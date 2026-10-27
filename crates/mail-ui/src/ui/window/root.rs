//! The root of a conversation's own window: the reader, and what it needs to act.
//!
//! quire gives this VirtualDom the same root contexts as the first window: the client, the look,
//! the Spaces, the directories and the frames' network. Everything a window keeps for itself is
//! made here again: its `Shell` (so its consent, its find and its undo are its own), its hover
//! and motion state, its toast host and a composer desk for a reply.

use super::note::left;
use crate::ui::actions::{self, Heard};
use crate::ui::app::Frame;
use crate::ui::compose::{self, SendPill};
use crate::ui::intent::{Intent, Scope, intent, on_thread};
use crate::ui::reading::{ReaderIn, reading_column};
use crate::ui::space::Spaces;
use crate::ui::style::STYLE;
use crate::ui::view::{Shell, Shortcut};
use crate::ui::window_core::{use_window_core, use_window_desk};
use chordkit::StandardAction;
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds_settings::use_environment;
use mail_domain::*;
use std::sync::Arc;

/// The conversation a window shows, as a root context: how a test's harness, which renders a
/// root without props, names it ([`message_root`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageOpen(pub ThreadId);

/// The window as quire opens it: the settings watched as the first window's root watches them,
/// then [`MessageShell`].
#[component]
pub(super) fn MessageWindow(thread: ThreadId) -> Element {
    let desktop = crate::ui::launch::DesktopSettings::current();
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(desktop.store(), desktop.prefs.clone(), spawner);
    use_context_provider(|| environment);
    crate::ui::prefs::use_window_settings();
    rsx! { MessageShell { thread } }
}

/// The window as a test drives it: [`MessageShell`] on the conversation its [`MessageOpen`]
/// context names, without the watch on the settings directory.
pub fn message_root() -> Element {
    let MessageOpen(thread) = consume_context::<MessageOpen>();
    rsx! { MessageShell { thread } }
}

/// One conversation's reader, in a window of its own.
#[component]
pub(in crate::ui) fn MessageShell(thread: ThreadId) -> Element {
    // The provider icons, as the first window holds them.
    let loaded = try_consume_context::<mail_client::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    crate::ui::provider_chip::use_fetch_missing(icons);
    crate::ui::host::use_window_host();
    // The settings its root watches (brand logos, spelling, server search), or, drawn by a test,
    // read from its config directory.
    let _ = crate::ui::prefs::use_prefs(
        try_consume_context::<crate::ui::appearance::WindowDirs>().as_ref(),
    );
    // The window's keymap, with mailo's actions in it: this window is its own root.
    let core = use_window_core();
    let keys = core.keys;
    let shell = use_signal(|| {
        let client = crate::ui::live::client();
        let mut shell = Shell {
            appearance: try_consume_context::<crate::ui::view::Appearance>().unwrap_or_default(),
            keymap: try_consume_context::<crate::ui::appearance::WindowDirs>()
                .map(|dirs| crate::ui::keymap::load(&dirs.config))
                .unwrap_or_default(),
            labels: client.query().known_labels(),
            accounts: client.compose().sending_accounts(),
            ..Shell::default()
        };
        // Opened as a click opens it: nothing consented to, nothing found.
        shell.open(thread);
        shell
    });
    // The Spaces, read and never written: the first window owns the file.
    let spaces = use_signal(|| {
        try_consume_context::<crate::ui::appearance::WindowDirs>()
            .map(|dirs| crate::ui::space::load(&dirs))
            .or_else(try_consume_context::<Spaces>)
            .unwrap_or_else(|| crate::ui::space::first_run(&[]))
    });
    // A reply's desk. Handed no directories, so nothing here writes Today or the settings,
    // which are the first window's.
    let today = use_signal(crate::ui::today::Today::default);
    let desk = use_window_desk(core, shell, spaces, None, today, None);
    // Where the conversation was when the window opened, for [`left`].
    let opened = use_hook(|| {
        let client = crate::ui::live::client();
        client
            .stored()
            .thread(thread)
            .map(|loaded| loaded.summary.mailboxes)
            .unwrap_or(MailboxSet::empty())
    });
    // Drawn again whenever either window moves the client.
    let _ = crate::ui::live::moved();
    let client = crate::ui::live::client();
    let summary = client
        .stored()
        .thread(thread)
        .ok()
        .map(|loaded| loaded.summary);
    let note = summary
        .as_ref()
        .and_then(|summary| left(opened, summary.mailboxes));

    let on_key = move |event: Event<KeyboardData>| {
        let key = event.key().to_string();
        if shell.read().preview.is_some() {
            crate::ui::reading::peek_key(shell, keys, &event);
            return;
        }
        // Escape closes ⌘F's bar first, as in the first window.
        if key == "Escape" && shell.read().find.is_some() && shell.read().composing.is_none() {
            crate::ui::reading::find::close(shell);
            return;
        }
        if shell.read().raw.is_some() {
            if key == "Escape" {
                crate::ui::reading::close_raw(shell);
            }
            return;
        }
        let typing = shell.read().composing.is_some();
        let heard = actions::heard(keys, &event, typing);
        let intent = intent(heard, Scope::Window);
        if crate::ui::motion::key(
            &key,
            heard == Some(Heard::Standard(StandardAction::Undo)),
            typing,
            shell,
        ) {
            return;
        }
        match intent {
            Some(Intent::Print) => {
                if let Some(job) = crate::ui::print::job_for(Some(thread)) {
                    crate::ui::print::print(job);
                }
                return;
            }
            Some(Intent::Find) => {
                crate::ui::reading::find::open(shell);
                return;
            }
            Some(Intent::FindStep(step)) => {
                crate::ui::reading::find::step(shell, step);
                return;
            }
            _ => {}
        }
        match intent {
            Some(Intent::Zoom(step)) => {
                crate::ui::reading::zoom(step);
                return;
            }
            Some(Intent::Raw(raw)) => {
                crate::ui::reading::ask_raw(shell, thread, raw);
                return;
            }
            _ => {}
        }
        // The mail keys and Esc; a chord of the main window's (search, Spaces) is not this one's.
        let Some(Intent::Mail(action)) = intent else {
            return;
        };
        let client = crate::ui::live::client();
        match action {
            Shortcut::Back => {
                if shell.read().composing.is_some() {
                    compose::park_current(desk, shell);
                }
            }
            Shortcut::TogglePin | Shortcut::Reply | Shortcut::ReplyAll | Shortcut::Forward => {
                on_thread(&client, shell, thread, action);
            }
            Shortcut::ToggleMute => {
                crate::ui::picks::mute_all(&client, shell, &[thread]);
            }
            _ => {
                // The same gesture the first window's keys make, over the one conversation
                // there is: one undo, one toast.
                let listed: Vec<ThreadSummary> = client
                    .stored()
                    .thread(thread)
                    .map(|loaded| vec![loaded.summary])
                    .unwrap_or_default();
                crate::ui::picks::act_on_picked(&client, shell, action, &listed);
            }
        }
    };

    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app message-window",
                tabindex: "0",
                onmounted: crate::ui::host::Host::app_mounted,
                onkeydown: on_key,
                if shell.read().preview.is_some() {
                    crate::ui::reading::PeekView { shell }
                }
                if shell.read().raw.is_some() {
                    crate::ui::reading::RawSheet { shell }
                }
                div { class: "card",
                    section { class: "reader",
                        if let Some(note) = note {
                            InlineBanner { severity: Severity::Info, text: note }
                        }
                        {reading_column(Some(thread), shell, ReaderIn::Window)}
                        crate::ui::hover::LinkPill {}
                    }
                    SendPill { shell }
                }
                crate::ui::motion::Toast { shell }
            }
        }
    }
}
