//! The window's toast host: what its buttons do.
//!
//! Every toast is quire's, through the window root's host: an undo, plain words, and the one
//! action that is not an undo (leaving a mailing list offers to archive what it already sent;
//! blocking a sender offers to take the block back). The handlers belong to the scope that made
//! them and must outlive the toast, which is why this component, mounted as long as the list,
//! makes them.

use super::{Follow, Toasts, motion, undo_by};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::stack::toast_hub::UndoToken;
use mail_core::undo::UndoHandle;
use mail_domain::RuleId;
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
        let on_unblock = EventHandler::new(move |()| {
            let mut said = state.toast;
            let offered = said.peek().clone();
            said.set(None);
            if let Some(Follow::Unblock { rule, sender }) = offered.map(|said| said.follow) {
                unblock(revision, rule, &sender);
            }
        });
        let mut toasts = state.toasts;
        toasts.set(Some(Toasts {
            hub,
            on_undo,
            on_archive,
            on_unblock,
        }));
    });
    rsx! {}
}

/// Take a block back: the rule it made is forgotten, and the toast says so.
fn unblock(mut revision: Signal<u64>, rule: RuleId, sender: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    let text = match mail_core::rules::block::unblock(&store, rule) {
        Ok(()) => format!("Unblocked {sender}"),
        Err(why) => format!("Could not unblock {sender}: {why}"),
    };
    revision += 1;
    super::tell(text, Follow::Nothing);
}
