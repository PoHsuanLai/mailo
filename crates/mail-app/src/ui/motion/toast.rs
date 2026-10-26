//! The window's toast host: what its buttons do.
//!
//! Every toast is quire's, through the window root's host: an undo, plain words, and the one
//! action that is not an undo (leaving a mailing list offers to archive what it already sent;
//! blocking a sender offers to take the block back; a sign-in in the browser that did not work
//! offers to try again; a rule or a key deleted in Settings offers to put it back). The handlers belong to the scope that made
//! them and must outlive the toast, which is why this component, mounted as long as the list,
//! makes them.

use super::{Follow, Toasts, motion, undo_by};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::stack::toast_hub::UndoToken;
use mail_core::undo::UndoHandle;

#[component]
pub(in crate::ui) fn Toast(shell: Signal<Shell>) -> Element {
    let Some(state) = motion() else {
        return rsx! {};
    };
    let hub = use_toasts();
    use_hook(move || {
        let client = crate::ui::live::client();
        let on_undo = EventHandler::new(move |token: UndoToken| {
            undo_by(&client, shell, Some(state), UndoHandle(token.0));
        });
        let on_archive = EventHandler::new(move |()| {
            let mut said = state.toast;
            let offered = said.peek().clone();
            said.set(None);
            if let Some(Follow::ArchiveFrom { sender, list }) = offered.map(|said| said.follow) {
                let client = crate::ui::live::client();
                crate::ui::unsubscribe::archive_list(&client, shell, &sender, &list);
            }
        });
        let on_sign_in = EventHandler::new(move |()| {
            let mut said = state.toast;
            let offered = said.peek().clone();
            said.set(None);
            if let Some(Follow::SignIn(retry)) = offered.map(|said| said.follow) {
                crate::ui::add_account::retry(retry);
            }
        });
        let on_restore = EventHandler::new(move |()| {
            let mut said = state.toast;
            let offered = said.peek().clone();
            said.set(None);
            if let Some(Follow::Restore(restore)) = offered.map(|said| said.follow) {
                let client = crate::ui::live::client();
                if let Err(why) = restore.put_back(&client) {
                    super::tell(why, Follow::Nothing);
                }
            }
        });
        let mut toasts = state.toasts;
        toasts.set(Some(Toasts {
            hub,
            on_undo,
            on_archive,
            on_sign_in,
            on_restore,
        }));
    });
    rsx! {}
}
