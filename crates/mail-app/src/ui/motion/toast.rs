//! The toast: quire's, through the window root's host, with an undo that takes back the op it
//! named; and mailo's own for the one toast whose follow-up is not an undo.
//!
//! quire's host draws the pull tab and follows the pointer itself. What it needs from here is a
//! handler that outlives the toast, which is why this component, mounted as long as the list,
//! makes it.

use super::{Follow, Toasts, motion, undo_by};
use crate::undo::UndoHandle;
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{Button, ButtonVariant, Icon, UndoToken, use_toasts};
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
        let mut toasts = state.toasts;
        toasts.set(Some(Toasts { hub, on_undo }));
    });
    let Some(said) = state.toast.read().clone() else {
        return rsx! {};
    };
    match said.follow {
        Follow::ArchiveFrom { sender, list } => rsx! {
            div { class: "toast", role: "status",
                span { "{said.text}" }
                Button {
                    variant: ButtonVariant::Mini,
                    label: "Archive all from this list".to_owned(),
                    icon: Some(Icon::Archive),
                    onclick: move |_| {
                        let store = consume_context::<Arc<SqliteStore>>();
                        crate::ui::unsubscribe::archive_list(&store, shell, revision, &sender, &list);
                    },
                }
            }
        },
        Follow::Unblock { rule, sender } => rsx! {
            div { class: "toast", role: "status",
                span { "{said.text}" }
                Button {
                    variant: ButtonVariant::Mini,
                    label: "Undo".to_owned(),
                    icon: Some(Icon::Undo),
                    onclick: move |_| unblock(revision, rule, &sender),
                }
            }
        },
        Follow::Undo(_) | Follow::Nothing => rsx! {},
    }
}

/// Take a block back: the rule it made is forgotten, and the toast says so.
fn unblock(mut revision: Signal<u64>, rule: RuleId, sender: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    let text = match crate::rules::block::unblock(&store, rule) {
        Ok(()) => format!("Unblocked {sender}"),
        Err(why) => format!("Could not unblock {sender}: {why}"),
    };
    revision += 1;
    super::tell(text, Follow::Nothing);
}
