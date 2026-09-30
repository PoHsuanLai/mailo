//! The window's toast host: what its buttons do.
//!
//! Every toast is quire's, through the window root's host: an undo, plain words, and the one
//! offer that is not an undo (leaving a mailing list offers to archive what it already sent, a
//! button on the toast). The handlers belong to the scope that made them and must outlive the
//! toast, which is why this component, mounted as long as the list, makes them.

use super::{Follow, Toasts, motion, undo_by};
use crate::undo::UndoHandle;
use crate::view::Shell;
use dioxus::prelude::*;
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
        let on_archive = EventHandler::new(move |()| {
            let mut said = state.toast;
            let offered = said.peek().clone();
            said.set(None);
            if let Some(Follow::ArchiveFrom { sender, list }) = offered.map(|said| said.follow) {
                let store = consume_context::<Arc<SqliteStore>>();
                crate::ui::unsubscribe::archive_list(&store, shell, revision, &sender, &list);
            }
        });
        let mut toasts = state.toasts;
        toasts.set(Some(Toasts {
            hub,
            on_undo,
            on_archive,
        }));
    });
    rsx! {}
}
