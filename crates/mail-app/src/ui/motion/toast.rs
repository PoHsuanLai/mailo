//! The undo toast, and the one offer that is not an undo.
//!
//! An undo and plain words are quire's toast, through the window root's host: what it needs from
//! here is a handler that outlives the toast, which is why this component, mounted as long as
//! the list, makes it. Leaving a mailing list offers something a toast cannot carry (archive
//! what it already sent), so that is a quire `Alert`, which waits for an answer.

use super::{Follow, Toasts, motion, undo_by};
use crate::undo::UndoHandle;
use crate::view::Shell;
use dioxus::prelude::*;
use ds::components::overlays::alert_model::{AlertButton, AlertRole};
use ds::prelude::*;
use ds::stack::toast_hub::UndoToken;
use mail_store::SqliteStore;
use std::sync::Arc;

#[component]
pub(in crate::ui) fn Toast(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let Some(state) = motion() else {
        return rsx! {};
    };
    let hub = use_toasts();
    use_hook(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        let on_undo = EventHandler::new(move |token: UndoToken| {
            undo_by(&store, shell, revision, Some(state), UndoHandle(token.0));
        });
        let mut toasts = state.toasts;
        toasts.set(Some(Toasts { hub, on_undo }));
    });
    let mut said_signal = state.toast;
    let Some(said) = said_signal.read().clone() else {
        return rsx! {};
    };
    let Follow::ArchiveFrom { sender, list } = said.follow else {
        return rsx! {};
    };
    let archive = EventHandler::new(move |()| {
        let store = consume_context::<Arc<SqliteStore>>();
        crate::ui::unsubscribe::archive_list(&store, shell, revision, &sender, &list);
        said_signal.set(None);
    });
    let not_now = EventHandler::new(move |()| said_signal.set(None));
    rsx! {
        Alert {
            title: said.text,
            message: Some(TextLine::from("Archive everything it already sent?")),
            buttons: vec![
                AlertButton::new("Archive All", AlertRole::Normal, archive),
                AlertButton::new("Cancel", AlertRole::Cancel, not_now),
            ],
        }
    }
}
