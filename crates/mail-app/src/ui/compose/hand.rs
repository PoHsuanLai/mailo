//! What a compose window hands the main window, and the main window's end of it.
//!
//! A new message is written in a window of its own (`ui/window/compose.rs`), and the main window
//! keeps everything that outlives that window: Today and its parked pages, the send pill and its
//! Undo, and the undo of a deleted draft. A `Signal` belongs to one VirtualDom, so the compose
//! window's desk hands these over through a [`Hub`], one for the app, which every window is given
//! as a root context (as it is given `Revisions`) and which the main window alone reads.

use std::sync::{Arc, Mutex};

use dioxus::prelude::*;
use mail_core::undo::Undo;
use mail_core::{SqliteStore, Store};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::desk::{self, Desk, Outgoing};
use super::page::Page;
use crate::ui::view::Shell;

/// One thing a compose window hands the main window.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) enum Handed {
    /// The window closed (or Esc, or Keep for Later) with the draft still being written: saved,
    /// and to be listed in Today exactly as it was left.
    Parked(Page),
    /// Send queued it: the main window's pill shows it and offers Undo.
    Queued(Outgoing),
    /// ⋯ › Open in Main Window: the page, to be drawn in the main window's reader column.
    ToMain(Page),
    /// Delete Draft's undo, for the main window's toast and ⌘Z.
    Undo(Undo),
}

/// The app's one way from a compose window to the main window. Cloning shares it.
#[derive(Clone)]
pub struct Hub(Arc<Shared>);

struct Shared {
    give: UnboundedSender<Handed>,
    /// Taken once, by the main window.
    take: Mutex<Option<UnboundedReceiver<Handed>>>,
}

impl Hub {
    /// Nothing handed yet.
    pub fn new() -> Self {
        let (give, take) = unbounded_channel();
        Hub(Arc::new(Shared {
            give,
            take: Mutex::new(Some(take)),
        }))
    }

    /// Hand `handed` to the main window. The main window closes every compose window as it goes
    /// (`use_handed`), so there is always one to take it.
    pub(in crate::ui) fn hand(&self, handed: Handed) {
        let _ = self.0.give.send(handed);
    }

    /// The main window's end, once.
    fn receiver(&self) -> Option<UnboundedReceiver<Handed>> {
        self.0.take.lock().ok()?.take()
    }
}

impl Default for Hub {
    fn default() -> Self {
        Hub::new()
    }
}

impl PartialEq for Hub {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hub")
    }
}

/// The main window's end: provide the app's [`Hub`] to this window (its own, when it was given
/// none) and take in what compose windows hand it. Called once, from `App`.
pub(in crate::ui) fn use_handed(desk: Desk, shell: Signal<Shell>, revision: Signal<u64>) {
    let hub = use_hook(|| try_consume_context::<Hub>().unwrap_or_default());
    use_context_provider(|| hub.clone());
    let window = ds_blitz::use_window_handle();
    // No compose window outlives this one: its hand-overs would go nowhere.
    use_drop(crate::ui::window::close_all);
    use_future(move || {
        let hub = hub.clone();
        let window = window.clone();
        async move {
            let Some(mut handed) = hub.receiver() else {
                return;
            };
            while let Some(one) = handed.recv().await {
                let to_main = matches!(one, Handed::ToMain(_));
                take(desk, shell, revision, one);
                // The page moved here: this window comes forward with it.
                if to_main && let Some(window) = &window {
                    window.focus();
                }
            }
        }
    });
}

/// Do in the main window what a compose window handed it.
fn take(mut desk: Desk, mut shell: Signal<Shell>, mut revision: Signal<u64>, handed: Handed) {
    match handed {
        Handed::Parked(page) => desk::keep_page(desk, page),
        Handed::Queued(outgoing) => {
            let draft = outgoing.draft;
            desk.outbox.set(Some(outgoing));
            desk::unpark(desk, draft);
        }
        Handed::ToMain(page) => {
            let store = consume_context::<Arc<SqliteStore>>();
            let Ok(stored) = store.draft(page.draft) else {
                return;
            };
            desk::unpark(desk, page.draft);
            {
                let mut parked = desk.parked.write();
                parked.retain(|kept| kept.draft != page.draft);
                parked.push(page);
            }
            shell.write().compose_in_place(&stored);
        }
        Handed::Undo(undo) => desk::offer_undo(shell, undo),
    }
    revision += 1;
}
