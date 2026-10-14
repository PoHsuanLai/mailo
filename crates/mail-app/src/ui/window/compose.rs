//! A new message in a small window of its own (Arc's Little Arc): New Message, ⌘N, ⌘K's Compose,
//! a `mailto:` link, a forward, a template, and a draft reopened from Today or Drafts all open the
//! composer here, so the reader stays readable while you write. A reply stays under its thread.
//!
//! Every way in only asks a window's `Shell` to compose (`Shell::compose`); [`use_compose_windows`]
//! sees a new message there and opens its window instead of drawing it in the reader column
//! ([`crate::ui::compose::to_window`]). Where no window can open (quire's harness, a snapshot) the
//! message stays in place, as "Open in Main Window" would put it.
//!
//! The window is quire's (`ds_blitz::open_window_with`), as a conversation's is: it reads the
//! same store, and hands what outlives it (Today's parked page, the send pill, a deleted draft's
//! undo, the page itself for "Open in Main Window") to the main window through the app's
//! [`Hub`]. Closing it parks the draft; Send, Esc, Delete Draft and Open in Main Window close it
//! once its composer has closed. One window per draft: asking for a draft already open raises
//! its window.

use crate::ui::actions::{self, Heard};
use crate::ui::app::Frame;
use crate::ui::compose::{self, ComposerPage, Desk, Hub, Page};
use crate::ui::copy;
use crate::ui::space::Spaces;
use crate::ui::style::STYLE;
use crate::ui::view::{Shell, Shortcut};
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds_blitz::{CloseAnswer, WindowHandle, WindowSpec};
use ds_settings::use_environment;
use mail_core::{SqliteStore, Store};
use mail_domain::DraftId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// A draft to open in a compose window, and the window's title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeAsk {
    pub draft: DraftId,
    pub title: String,
}

/// Whatever opens a compose window: quire's event loop in the launched window, a recorder in a
/// test.
pub trait OpenCompose: Send + Sync + 'static {
    /// Open `ask`'s draft in a compose window.
    fn open(&self, ask: ComposeAsk);
}

/// Where a new message's window goes, as a root context. Without one it is quire's
/// `open_window_with`. Cloning shares it.
#[derive(Clone)]
pub struct ComposeWindows(pub Arc<dyn OpenCompose>);

impl std::fmt::Debug for ComposeWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ComposeWindows")
    }
}

/// The draft a compose window holds, as a root context: how a test's harness, which renders a
/// root without props, names it ([`compose_root`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeOpen(pub DraftId);

/// The size a compose window opens at, in logical pixels, and the least it may be made.
const SIZE: (u32, u32) = (640, 560);
const LEAST: (u32, u32) = (420, 320);

thread_local! {
    /// The compose windows open, by draft. Every window of the app draws on this one thread, so
    /// whichever window asks for a draft finds the window already open on it.
    static OPEN: RefCell<HashMap<DraftId, WindowHandle>> = RefCell::new(HashMap::new());
}

/// Whether a compose window took the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opened {
    Window,
    /// No window could open: the draft is drawn in place.
    Here,
}

/// A compose window's title: the subject, or "New Message" until there is one.
pub(in crate::ui) fn title(subject: &str) -> String {
    match subject.trim() {
        "" => copy::titles::NEW_MESSAGE.to_owned(),
        said => said.to_owned(),
    }
}

/// Open `draft` in a compose window, `page` being its page exactly as it was left when there is
/// one (parked, or taken back from the outbox), or raise the window already open on it.
fn open(draft: DraftId, page: Option<Page>, subject: &str) -> Opened {
    let open = OPEN.with(|open| open.borrow().get(&draft).cloned());
    if let Some(handle) = open.filter(|handle| super::raise(Some(handle.life()))) {
        handle.focus();
        return Opened::Window;
    }
    let ask = ComposeAsk {
        draft,
        title: title(subject),
    };
    if let Some(windows) = try_consume_context::<ComposeWindows>() {
        windows.0.open(ask);
        return Opened::Window;
    }
    let Some(hub) = try_consume_context::<Hub>() else {
        return Opened::Here;
    };
    let spec = WindowSpec::new(
        ask.title,
        ds_blitz::WindowSize::new(SIZE.0, SIZE.1).with_least(LEAST.0, LEAST.1),
    );
    match ds_blitz::open_window_with(spec, ComposeWindow, ComposeWindowProps { draft, page, hub }) {
        Ok(handle) => {
            OPEN.with(|open| open.borrow_mut().insert(draft, handle));
            Opened::Window
        }
        Err(_) => Opened::Here,
    }
}

/// Close every compose window, each keeping its draft as it was left. The main window does this
/// as it closes: what a compose window hands over (Today, the send pill, Delete Draft's undo) has
/// nowhere to go once the main window is gone.
pub(in crate::ui) fn close_all() {
    let open: Vec<WindowHandle> = OPEN.with(|open| {
        open.borrow_mut()
            .drain()
            .map(|(_, handle)| handle)
            .collect()
    });
    for handle in open {
        handle.close();
    }
}

/// Send a new message this window is asked to compose to a window of its own. Called once by each
/// window that composes: the main window and a conversation's (whose forward is a new message).
pub(in crate::ui) fn use_compose_windows(shell: Signal<Shell>, desk: Desk) {
    use_effect(move || {
        let Some(draft) = compose::to_window(&shell.read()) else {
            return;
        };
        let mut desk = desk;
        let mut shell = shell;
        // The page as it was left, when it was parked here or taken back from the outbox.
        let page = desk
            .parked
            .peek()
            .iter()
            .find(|page| page.draft == draft)
            .cloned();
        let subject = match &page {
            Some(page) => page.subject.clone(),
            None => {
                let store = consume_context::<Arc<SqliteStore>>();
                store
                    .draft(draft)
                    .map(|stored| stored.subject)
                    .unwrap_or_default()
            }
        };
        match open(draft, page, &subject) {
            Opened::Window => {
                desk.parked.write().retain(|page| page.draft != draft);
                compose::unpark(desk, draft);
                shell.write().close_composer();
            }
            Opened::Here => {
                if let Some(open) = shell.write().composing.as_mut() {
                    open.in_place = true;
                }
            }
        }
    });
}

/// The window as quire opens it: the settings watched as the first window's root watches them,
/// then [`ComposeShell`].
#[component]
fn ComposeWindow(draft: DraftId, page: Option<Page>, hub: Hub) -> Element {
    let desktop = crate::ui::launch::DesktopSettings::current();
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(desktop.store(), desktop.prefs.clone(), spawner);
    use_context_provider(|| environment);
    crate::ui::prefs::use_window_settings();
    rsx! { ComposeShell { draft, page, hub } }
}

/// The window as a test drives it: [`ComposeShell`] on the draft its [`ComposeOpen`] context
/// names, handing to the [`Hub`] it was given (a fresh one when none), without the watch on the
/// settings directory.
pub fn compose_root() -> Element {
    let ComposeOpen(draft) = consume_context::<ComposeOpen>();
    let hub = try_consume_context::<Hub>().unwrap_or_default();
    rsx! { ComposeShell { draft, page: None, hub } }
}

/// One new message's page, in a window of its own.
#[component]
fn ComposeShell(draft: DraftId, page: Option<Page>, hub: Hub) -> Element {
    // The provider icons, as the first window holds them.
    let loaded = try_consume_context::<mail_core::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    crate::ui::provider_chip::use_fetch_missing(icons);
    crate::ui::host::use_window_host();
    let _ = crate::ui::prefs::use_prefs(
        try_consume_context::<crate::ui::appearance::WindowDirs>().as_ref(),
    );
    let keys = actions::use_registered();
    let shell = use_signal(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        let mut shell = Shell {
            appearance: try_consume_context::<crate::ui::view::Appearance>().unwrap_or_default(),
            keymap: try_consume_context::<crate::ui::appearance::WindowDirs>()
                .map(|dirs| crate::ui::keymap::load(&dirs.config))
                .unwrap_or_default(),
            labels: mail_core::query::known_labels(&store),
            accounts: mail_core::compose::sending_accounts(&store),
            ..Shell::default()
        };
        if let Ok(stored) = store.draft(draft) {
            shell.compose_in_place(&stored);
        }
        shell
    });
    let revision = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_revision(revision);
    // The Spaces, read and never written: the first window owns the file.
    let spaces = use_signal(|| {
        try_consume_context::<crate::ui::appearance::WindowDirs>()
            .map(|dirs| crate::ui::space::load(&dirs))
            .or_else(try_consume_context::<Spaces>)
            .unwrap_or_else(|| crate::ui::space::first_run(&[]))
    });
    crate::ui::frame::use_followed_configuration(shell, spaces, None, keys);
    // Handed no directories: Today, the pill and the undo are the main window's, through `hub`.
    let today = use_signal(crate::ui::today::Today::default);
    let desk = compose::use_desk(today, spaces, None, Some(hub));
    // The page as it was left, for the composer to open on.
    use_hook(|| {
        if let Some(page) = page {
            let mut parked = desk.parked;
            parked.write().push(page);
        }
    });
    compose::use_test_dictionaries();
    crate::ui::hover::use_hover();
    crate::ui::motion::use_motion();

    // The window's own close (its frame, the compositor) parks the draft: nothing typed is lost.
    ds_blitz::use_close_request(move || {
        compose::park_current(desk, shell);
        CloseAnswer::Close
    });
    // The composer closed (Send, Esc, Delete Draft, Open in Main Window): so does the window.
    let window = ds_blitz::use_window_handle();
    use_effect(move || {
        if shell.read().composing.is_some() {
            return;
        }
        OPEN.with(|open| open.borrow_mut().remove(&draft));
        if let Some(window) = &window {
            window.close();
        }
    });

    let on_key = move |event: Event<KeyboardData>| {
        let heard = actions::heard(keys, &event, true);
        // Esc with the keyboard outside the page parks the draft, as it does inside.
        if heard == Some(Heard::Mail(Shortcut::Back)) {
            compose::park_current(desk, shell);
        }
    };

    let composing = shell.read().composing.as_ref().map(|open| open.draft);
    rsx! {
        Frame { spaces,
            style { {STYLE} }
            ComposeTitle { desk }
            div { class: "app compose-window",
                tabindex: "0",
                onmounted: crate::ui::host::Host::app_mounted,
                onkeydown: on_key,
                div { class: "card",
                    section { class: "reader",
                        if let Some(draft) = composing {
                            ComposerPage { key: "{draft}", draft, shell, revision }
                        }
                        crate::ui::hover::LinkPill {}
                    }
                }
                crate::ui::motion::Toast { shell, revision }
            }
        }
    }
}

/// Keeps the window's title at the page's subject, or "New Message". A component of its own, so
/// a keystroke redraws only this.
#[component]
fn ComposeTitle(desk: Desk) -> Element {
    let page = *desk.current.read();
    let subject = page
        .and_then(|page| page.try_read().ok().map(|page| page.subject.clone()))
        .unwrap_or_default();
    ds::window::host::use_window_title(title(&subject));
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_subject_and_new_message_until_there_is_one() {
        const CASES: &[(&str, &str)] = &[
            ("Lunch on Thursday", "Lunch on Thursday"),
            ("  Fwd: plans \t", "Fwd: plans"),
            ("", "New Message"),
            ("   ", "New Message"),
        ];
        for (subject, want) in CASES {
            assert_eq!(title(subject), *want, "{subject:?}");
        }
    }
}
