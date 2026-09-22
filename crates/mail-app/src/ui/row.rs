//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader and carries the hover strip. Split from [`super::app`] (`CONVENTIONS.md` §8).

use super::menus::{LabelMenu, SnoozeMenu};
use super::ops::{apply_op, composes, start_composing};
use super::text::{draft_state, label, sender};
use crate::view::{Shell, hover_actions};
use chrono::Local;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// A draft, as a row. Clicking it opens the composer on that draft.
#[component]
pub(super) fn DraftRow(draft: Draft, shell: Signal<Shell>) -> Element {
    let id = draft.id;
    let subject = if draft.subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        draft.subject.clone()
    };
    let who = crate::view::join_addresses(&draft.to);
    let state = draft_state(&draft.state);
    let when = crate::view::listed(draft.updated, chrono::Utc::now(), &Local);
    rsx! {
        div {
            key: "{id}",
            class: "row",
            onclick: move |_| {
                let store = consume_context::<Arc<SqliteStore>>();
                if let Ok(draft) = store.draft(id) {
                    shell.write().compose(&draft);
                }
            },
            span { class: "who", if who.is_empty() { "(no recipient)" } else { "{who}" } }
            span { class: "subject", "{subject}" }
            span { class: "when", "{state} · {when}" }
        }
    }
}

/// A conversation, as a row, including the hover strip and whichever menu it has open.
#[component]
pub(super) fn Row(summary: ThreadSummary, shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let id = summary.id;
    let unread = summary.read == ReadState::Unread;
    let who = sender(&summary);
    let when = crate::view::listed(summary.last_date, chrono::Utc::now(), &Local);
    let subject = summary.subject.clone();
    let actions = hover_actions(&summary);
    rsx! {
        div {
            key: "{id}",
            class: if unread { "row unread" } else { "row" },
            onclick: move |_| shell.write().open(id),
            span { class: "who", "{who}" }
            span { class: "subject", "{subject}" }
            span { class: "when", "{when}" }
            span { class: "hover",
                for kind in actions {
                    button {
                        key: "{kind:?}",
                        onclick: move |e: Event<MouseData>| {
                            // Without this the click also opens the thread.
                            e.stop_propagation();
                            let store = consume_context::<Arc<SqliteStore>>();
                            // A label needs a payload no button can
                            // carry, so this one opens a menu instead of
                            // performing anything.
                            if kind == OpKind::AddLabel {
                                let already =
                                    shell.peek().labelling == Some(id);
                                shell.write().labelling =
                                    if already { None } else { Some(id) };
                                return;
                            }
                            // Snooze needs a time, which is the same shape
                            // of payload as a label and gets the same
                            // answer: a menu rather than a guess.
                            if kind == OpKind::Snooze {
                                let already =
                                    shell.peek().snoozing == Some(id);
                                shell.write().snoozing =
                                    if already { None } else { Some(id) };
                                return;
                            }
                            match composes(kind) {
                                Some(what) => {
                                    match start_composing(&store, id, what) {
                                        Ok(draft) => {
                                            shell.write().compose(&draft);
                                            revision += 1;
                                        }
                                        Err(why) => {
                                            // Nowhere else to say it yet:
                                            // the composer that would show
                                            // a notice is what failed to
                                            // open.
                                            eprintln!("reply: {why}");
                                        }
                                    }
                                }
                                None => {
                                    if apply_op(&store, id, kind) {
                                        revision += 1;
                                    }
                                }
                            }
                        },
                        "{label(kind)}"
                    }
                }
            }
            if shell.read().snoozing == Some(id) {
                SnoozeMenu { id, shell, revision }
            }
            if shell.read().labelling == Some(id) {
                LabelMenu { id, summary, shell, revision }
            }
        }
    }
}
