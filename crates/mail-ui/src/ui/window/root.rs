//! The root of a conversation's own window: the reader, and what it needs to act.
//!
//! quire gives this VirtualDom the same root contexts as the first window: the client, the look,
//! the Spaces, the directories and the frames' network. Everything a window keeps for itself is
//! made here again: its `Shell` (so its consent, its find and its undo are its own), its hover
//! and motion state, its toast host and a composer desk for a reply.

use super::note::left;
use crate::ui::app::Frame;
use crate::ui::compose::{self, SendPill};
use crate::ui::reading::{ReaderIn, reading_column};
use crate::ui::style::STYLE;
use crate::ui::window_core::{use_window_core, use_window_desk};
use chordkit::StandardAction;
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds_settings::use_environment;
use mail_domain::*;
use mail_ui_kit::actions::{self, Heard};
use mail_ui_kit::intent::{Intent, Scope, intent, on_thread};
use mail_ui_kit::space::Spaces;
use mail_ui_kit::view::{Shell, Shortcut};
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
    mail_ui_kit::prefs::use_window_settings();
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
    mail_ui_kit::host::use_window_host();
    // The settings its root watches (brand logos, spelling, server search), or, drawn by a test,
    // read from its config directory.
    let _ = mail_ui_kit::prefs::use_prefs(
        try_consume_context::<mail_ui_kit::appearance::WindowDirs>().as_ref(),
    );
    // The window's keymap, with mailo's actions in it: this window is its own root.
    let core = use_window_core();
    let keys = core.keys;
    // The conversation and the labels, as the client keeps them read: the window reads no store.
    let every = mail_ui_kit::live::use_live(|| mail_client::query::Accounts);
    let read = mail_ui_kit::live::use_live(move || mail_client::query::Conversation(thread));
    let shell = use_signal(|| {
        let client = mail_ui_kit::live::client();
        let mut shell = Shell {
            appearance: try_consume_context::<mail_ui_kit::view::Appearance>().unwrap_or_default(),
            keymap: try_consume_context::<mail_ui_kit::appearance::WindowDirs>()
                .map(|dirs| mail_ui_kit::keymap::load(&dirs.config))
                .unwrap_or_default(),
            labels: every
                .peek()
                .value()
                .map(|accounts| crate::ui::sidebar::known_labels(accounts))
                .unwrap_or_default(),
            accounts: client.compose().sending_accounts(),
            ..Shell::default()
        };
        // Opened as a click opens it: nothing consented to, nothing found.
        shell.open(thread);
        shell
    });
    // The Spaces, read and never written: the first window owns the file.
    let spaces = use_signal(|| {
        try_consume_context::<mail_ui_kit::appearance::WindowDirs>()
            .map(|dirs| mail_ui_kit::space::load(&dirs))
            .or_else(try_consume_context::<Spaces>)
            .unwrap_or_else(|| mail_ui_kit::space::first_run(&[]))
    });
    // A reply's desk. Handed no directories, so nothing here writes Today or the settings,
    // which are the first window's.
    let today = use_signal(mail_ui_kit::today::Today::default);
    let desk = use_window_desk(core, shell, spaces, None, today, None);
    // Drawn again whenever the client reads the conversation again, here or from another window.
    let summary = conversation_of(&read.read(), thread);
    // Where the conversation was when the window opened, for [`left`]: where it was when the
    // client first had it.
    let opened = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None::<MailboxSet>)));
    if opened.get().is_none() {
        opened.set(summary.as_ref().map(|summary| summary.mailboxes));
    }
    let note = summary
        .as_ref()
        .and_then(|summary| left(opened.get()?, summary.mailboxes));

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
        if mail_ui_kit::motion::key(
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
                let _ = crate::ui::reading::find::open(shell);
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
        let client = mail_ui_kit::live::client();
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
                let listed: Vec<ThreadSummary> =
                    conversation_of(&read.peek(), thread).into_iter().collect();
                crate::ui::picks::act_on_picked(&client, shell, action, &listed);
            }
        }
    };

    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app message-window",
                tabindex: "0",
                onmounted: mail_ui_kit::host::Host::app_mounted,
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
                        mail_ui_kit::hover::LinkPill {}
                    }
                    SendPill { shell }
                }
                mail_ui_kit::motion::Toast { shell }
            }
        }
    }
}

/// The summary of `thread` as the client has it read, when it has it.
fn conversation_of(
    read: &mail_client::Snapshot<Option<mail_client::query::Messages>>,
    thread: ThreadId,
) -> Option<ThreadSummary> {
    match read {
        mail_client::Snapshot::Ready { value, .. } => value
            .as_ref()
            .as_ref()
            .filter(|loaded| loaded.summary.id == thread)
            .map(|loaded| loaded.summary.clone()),
        _ => None,
    }
}
