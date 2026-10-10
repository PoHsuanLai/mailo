//! The root of a conversation's own window: the reader, and what it needs to act.
//!
//! quire gives this VirtualDom the same root contexts as the first window: the store, the look,
//! the Spaces, the directories, the frames' network, and [`crate::ui::revisions::Revisions`].
//! Everything a window keeps for itself is made here again: its `Shell` (so its consent, its
//! find and its undo are its own), its `revision`, its hover and motion state, its toast host and
//! a composer desk for a reply.

use super::note::left;
use crate::ui::actions::{self, Heard};
use crate::ui::app::Frame;
use crate::ui::compose::{self, ComposerPage, PageKind, SendPill};
use crate::ui::ops::{Composes, apply_op, start_composing};
use crate::ui::reading::{Reader, ReaderIn};
use crate::ui::space::Spaces;
use crate::ui::style::STYLE;
use crate::ui::view::{Shell, Shortcut};
use chordkit::StandardAction;
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds_settings::use_environment;
use mail_core::{SqliteStore, Store};
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
    let loaded = try_consume_context::<mail_core::provider::icon::Loaded>().unwrap_or_default();
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
    let keys = actions::use_registered();
    let mut shell = use_signal(|| {
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
        // Opened as a click opens it: nothing consented to, nothing found.
        shell.open(thread);
        shell
    });
    let mut revision = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_revision(revision);
    // The Spaces, read and never written: the first window owns the file.
    let spaces = use_signal(|| {
        try_consume_context::<crate::ui::appearance::WindowDirs>()
            .map(|dirs| crate::ui::space::load(&dirs))
            .or_else(try_consume_context::<Spaces>)
            .unwrap_or_else(|| crate::ui::space::first_run(&[]))
    });
    crate::ui::frame::use_followed_configuration(shell, spaces, None, keys);
    // A reply's desk. Handed no directories, so nothing here writes Today or the settings,
    // which are the first window's.
    let today = use_signal(crate::ui::today::Today::default);
    let side_hidden = use_signal(|| true);
    let desk = compose::use_desk(today, spaces, None, side_hidden);
    compose::use_test_dictionaries();
    crate::ui::hover::use_hover();
    crate::ui::motion::use_motion();
    // Where the conversation was when the window opened, for [`left`].
    let opened = use_hook(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        store
            .thread(thread)
            .map(|loaded| loaded.summary.mailboxes)
            .unwrap_or(MailboxSet::empty())
    });
    // Drawn again whenever either window moves the store.
    let _ = revision();
    let store = consume_context::<Arc<SqliteStore>>();
    let summary = store.thread(thread).ok().map(|loaded| loaded.summary);
    let note = summary
        .as_ref()
        .and_then(|summary| left(opened, summary.mailboxes));

    let on_key = move |event: Event<KeyboardData>| {
        let key = event.key().to_string();
        if shell.read().viewing.is_some() {
            crate::ui::reading::viewer_key(shell, &key);
            return;
        }
        let typing = shell.read().composing.is_some();
        let heard = actions::heard(keys, &event, typing);
        if crate::ui::motion::key(
            &key,
            heard == Some(Heard::Standard(StandardAction::Undo)),
            typing,
            shell,
            revision,
        ) {
            return;
        }
        if heard == Some(Heard::Standard(StandardAction::Print)) {
            if let Some(job) = crate::ui::print::job_for(Some(thread)) {
                crate::ui::print::print(job);
            }
            return;
        }
        // The mail keys and Esc; a chord of the main window's (search, Spaces) is not this one's.
        let Some(Heard::Mail(action)) = heard else {
            return;
        };
        let store = consume_context::<Arc<SqliteStore>>();
        match action {
            // One conversation, and no list to step through or start another from.
            Shortcut::Next
            | Shortcut::Previous
            | Shortcut::ExtendNext
            | Shortcut::ExtendPrevious
            | Shortcut::Compose => {}
            Shortcut::Back => {
                if shell.read().composing.is_some() {
                    compose::park_current(desk, shell);
                    revision += 1;
                }
            }
            Shortcut::TogglePin => {
                if apply_op(&store, thread, OpKind::Pin) {
                    revision += 1;
                }
            }
            Shortcut::ToggleMute => {
                crate::ui::picks::mute_all(&store, shell, revision, &[thread]);
            }
            Shortcut::Reply | Shortcut::ReplyAll | Shortcut::Forward => {
                let what = match action {
                    Shortcut::Reply => Composes::Reply(ReplyScope::Sender),
                    Shortcut::ReplyAll => Composes::Reply(ReplyScope::All),
                    _ => Composes::Forward,
                };
                if let Ok(draft) = start_composing(&store, thread, what) {
                    shell.write().compose(&draft);
                    revision += 1;
                }
            }
            _ => {
                // The same gesture the first window's keys make, over the one conversation
                // there is: one undo, one toast.
                let listed: Vec<ThreadSummary> = store
                    .thread(thread)
                    .map(|loaded| vec![loaded.summary])
                    .unwrap_or_default();
                crate::ui::picks::act_on_picked(&store, shell, revision, action, &listed);
            }
        }
    };

    let composing = compose::composing(&shell.read());
    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app message-window",
                tabindex: "0",
                onmounted: crate::ui::host::Host::app_mounted,
                onkeydown: on_key,
                if shell.read().viewing.is_some() {
                    crate::ui::reading::AttachmentViewer { shell }
                }
                div { class: "card",
                    section { class: "reader",
                        if let Some(note) = note {
                            div { class: "left-note", role: "status", "{note}" }
                        }
                        match composing {
                            Some((draft, PageKind::Reply)) => rsx! {
                                Reader { thread, shell, revision, place: ReaderIn::Window,
                                    ComposerPage { key: "{draft}", draft, shell, revision }
                                }
                            },
                            // A forward is a message of its own: the page stands in for the reader.
                            Some((draft, _)) => rsx! { ComposerPage { key: "{draft}", draft, shell, revision } },
                            None => rsx! { Reader { thread, shell, revision, place: ReaderIn::Window } },
                        }
                        crate::ui::hover::LinkPill {}
                    }
                    SendPill { shell }
                }
                crate::ui::motion::Toast { shell, revision }
            }
        }
    }
}
