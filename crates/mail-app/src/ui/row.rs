//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader and carries the hover strip. Split from [`super::app`] (`CONVENTIONS.md` §8).

use super::icon::{Glyph, Icon};
use super::menus::{LabelMenu, SnoozeMenu};
use super::ops::{apply_op, composes, start_composing};
use super::text::{draft_state, label, sender};
use crate::provider::Provider;
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::view::Marks;
use crate::view::{Shell, hover_actions};
use chrono::Local;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// A draft, as a row. Clicking it opens the composer on that draft.
#[component]
pub(super) fn DraftRow(draft: Draft, shell: Signal<Shell>, index: usize) -> Element {
    let id = draft.id;
    let subject = if draft.subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        draft.subject.clone()
    };
    let who = crate::view::join_addresses(&draft.to);
    let who = if who.is_empty() {
        "(no recipient)".to_owned()
    } else {
        who
    };
    let state = draft_state(&draft.state);
    let when = crate::view::listed(draft.updated, chrono::Utc::now(), &Local);
    let delay = index.min(8);
    rsx! {
        li {
            key: "{id}",
            class: "row",
            role: "option",
            "data-read": "read",
            style: "--i:{delay}",
            onclick: move |_| {
                let store = consume_context::<Arc<SqliteStore>>();
                if let Ok(draft) = store.draft(id) {
                    shell.write().compose(&draft);
                }
            },
            span { class: "row-dot", span { class: "dot" } }
            div { class: "row-main",
                div { class: "row-from",
                    span { class: "nm", "{who}" }
                }
                div { class: "row-sub", "{subject}" }
                if shell.read().parts.snippet.shown() {
                    div { class: "row-snip", "{state}" }
                }
            }
            div { class: "row-tail",
                if shell.read().parts.time.shown() {
                    span { class: "row-time", "{when}" }
                }
            }
        }
    }
}

/// A conversation, as a row, including the hover strip and whichever menu it has open.
#[component]
pub(super) fn Row(
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    index: usize,
    chips: Vec<String>,
    via: Option<Provider>,
) -> Element {
    let id = summary.id;
    let unread = summary.read == ReadState::Unread;
    let starred = summary.star == Star::Starred;
    let who = sender(&summary);
    let when = crate::view::listed(summary.last_date, chrono::Utc::now(), &Local);
    let subject = summary.subject.clone();
    let snippet = summary.snippet.clone();
    let files = match summary.attachments {
        Attachments::Present { count } => Some(count),
        Attachments::None => None,
    };
    let actions: Vec<OpKind> = hover_actions(&summary)
        .into_iter()
        .filter(|kind| !matches!(kind, OpKind::Star | OpKind::Unstar))
        .collect();
    let selected = shell.read().open == Some(id);
    let delay = index.min(8);
    let mut pop = use_signal(|| false);
    rsx! {
        li {
            key: "{id}",
            class: "row",
            role: "option",
            aria_label: "Open {subject}",
            "data-read": if unread { "unread" } else { "read" },
            aria_selected: if selected { "true" } else { "false" },
            style: "--i:{delay}",
            onclick: move |_| shell.write().open(id),
            span { class: "row-dot", span { class: "dot" } }
            div { class: "row-main",
                div { class: "row-from",
                    span { class: "nm", "{who}" }
                    if shell.read().parts.provider.shown() {
                        if let Some(via) = via {
                            ViaChip { via, marks: shell.read().appearance.marks }
                        }
                    }
                }
                div { class: "row-sub", "{subject}" }
                if shell.read().parts.snippet.shown() && !snippet.is_empty() {
                    div { class: "row-snip", "{snippet}" }
                }
            }
            div { class: "row-tail",
                if shell.read().parts.time.shown() {
                    span { class: "row-time", "{when}" }
                }
                span { class: "chips",
                    if shell.read().parts.chips.shown() {
                        for name in chips {
                            span { key: "{name}", class: "chip", "data-chip": "{name}", "{name}" }
                        }
                    }
                    if let Some(count) = files {
                        span { class: "clip",
                            Glyph { icon: Icon::Paperclip, class: None }
                            "{count}"
                        }
                    }
                }
            }
            button {
                class: if pop() { "star pop" } else { "star" },
                "data-on": if starred { "true" } else { "false" },
                aria_label: if starred { "Unstar" } else { "Star" },
                aria_pressed: if starred { "true" } else { "false" },
                onclick: move |event| {
                    event.stop_propagation();
                    pop.set(true);
                    let store = consume_context::<Arc<SqliteStore>>();
                    let kind = if starred { OpKind::Unstar } else { OpKind::Star };
                    if apply_op(&store, id, kind) {
                        revision += 1;
                    }
                },
                onanimationend: move |_| pop.set(false),
                Glyph { icon: Icon::Star, class: None }
                span { class: if pop() { "sparks go" } else { "sparks" },
                    for angle in [0, 60, 120, 180, 240, 300] {
                        i { key: "{angle}", style: "--a:{angle}deg" }
                    }
                }
            }
            span { class: "strip",
                for (n, kind) in actions.iter().copied().enumerate() {
                    button {
                        key: "{kind:?}",
                        "data-op": "{kebab(kind)}",
                        aria_label: "{label(kind)}",
                        title: "{label(kind)}",
                        style: "--j:{n}",
                        onclick: move |event: Event<MouseData>| {
                            event.stop_propagation();
                            let store = consume_context::<Arc<SqliteStore>>();
                            if kind == OpKind::AddLabel {
                                let already = shell.peek().labelling == Some(id);
                                shell.write().labelling = if already { None } else { Some(id) };
                                return;
                            }
                            if kind == OpKind::Snooze {
                                let already = shell.peek().snoozing == Some(id);
                                shell.write().snoozing = if already { None } else { Some(id) };
                                return;
                            }
                            match composes(kind) {
                                Some(what) => match start_composing(&store, id, what) {
                                    Ok(draft) => {
                                        shell.write().compose(&draft);
                                        revision += 1;
                                    }
                                    Err(why) => eprintln!("reply: {why}"),
                                },
                                None => {
                                    if apply_op(&store, id, kind) {
                                        revision += 1;
                                    }
                                }
                            }
                        },
                        Glyph { icon: op_icon(kind), class: None }
                        span { class: "fly", "{fly(kind)}" }
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

#[component]
fn ViaChip(via: Provider, marks: Marks) -> Element {
    let short = via.short();
    let title = via.title();
    rsx! {
        span { class: "via", title: "{title}",
            ProvChip { provider: via, marks, place: ChipPlace::Row }
            "{short}"
        }
    }
}

/// `tomorrow` in [`crate::view::snooze_until`] is 09:00 local, which is what this says.
/// The mockup's card says 08:00; the menu and the command line both mean 09:00.
fn fly(kind: OpKind) -> String {
    match kind {
        OpKind::Snooze => "Tomorrow 09:00".to_owned(),
        OpKind::Archive => "Archive → out of Inbox".to_owned(),
        other => label(other).to_owned(),
    }
}

fn kebab(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "archive",
        OpKind::Trash => "trash",
        OpKind::Restore => "restore",
        OpKind::Spam => "spam",
        OpKind::MarkRead => "mark-read",
        OpKind::MarkUnread => "mark-unread",
        OpKind::Star => "star",
        OpKind::Unstar => "unstar",
        OpKind::AddLabel => "add-label",
        OpKind::RemoveLabel => "remove-label",
        OpKind::Snooze => "snooze",
        OpKind::Pin => "pin",
        OpKind::Reply => "reply",
        OpKind::ReplyAll => "reply-all",
        OpKind::Forward => "forward",
    }
}

fn op_icon(kind: OpKind) -> Icon {
    match kind {
        OpKind::Archive => Icon::Archive,
        OpKind::Trash => Icon::Trash,
        OpKind::Restore => Icon::Corner,
        OpKind::Spam => Icon::OctagonAlert,
        OpKind::MarkRead => Icon::MailOpen,
        OpKind::MarkUnread => Icon::Mail,
        OpKind::Star | OpKind::Unstar => Icon::Star,
        OpKind::AddLabel | OpKind::RemoveLabel => Icon::Tag,
        OpKind::Snooze => Icon::Clock,
        OpKind::Pin => Icon::Pin,
        OpKind::Reply => Icon::Reply,
        OpKind::ReplyAll => Icon::ReplyAll,
        OpKind::Forward => Icon::Forward,
    }
}
