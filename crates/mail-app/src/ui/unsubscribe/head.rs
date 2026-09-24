//! The reader head's "Unsubscribe", and the popover that says what it will do before it does it.

use super::super::compose::{Desk, show_queued};
use super::super::hover::{copy, url_spans};
use super::super::motion::{Follow, tell};
use super::{Ask, Bodies, Offer, ask, cached, client, leave, lookup, said};
use crate::unsubscribe::Outcome;
use dioxus::prelude::*;
use mail_domain::ThreadId;
use mail_store::SqliteStore;
use std::sync::Arc;

/// Where the popover is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    Closed,
    Asking,
    /// The way out is being taken; its button stays disabled so a second click cannot send twice.
    Working,
    /// It did not go, and why, in words.
    Failed(String),
}

/// "Unsubscribe", when the open thread offers a way out.
///
/// Draws nothing until the answer is known: the lookup reads a stored blob, so it runs on a
/// blocking thread and lands here, or is already in the cache from the last time this thread
/// was open. Keyed by the reader on the thread and its bodies, so an answer for one thread can
/// never be drawn, or acted on, under another.
#[component]
pub(in crate::ui) fn Leave(
    thread: ThreadId,
    bodies: Bodies,
    revision: Option<Signal<u64>>,
) -> Element {
    let mut known = use_signal({
        let bodies = bodies.clone();
        move || cached(thread, &bodies)
    });
    let _look = use_resource(move || {
        let bodies = bodies.clone();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            if known.peek().is_some() {
                return;
            }
            let offer = tokio::task::spawn_blocking(move || lookup(&store, thread, &bodies))
                .await
                .ok()
                .flatten();
            known.set(Some(offer));
        }
    });
    let mut phase = use_signal(|| Phase::Closed);
    let Some(Some(offer)) = known() else {
        return rsx! {};
    };
    let Some(asked) = ask(&offer) else {
        return rsx! {};
    };
    let open = phase() != Phase::Closed;
    let label = "Unsubscribe";
    rsx! {
        div { class: "leave",
            button {
                class: "mini",
                r#type: "button",
                aria_label: "{label}",
                aria_expanded: if open { "true" } else { "false" },
                onclick: move |_| {
                    let next = if *phase.peek() == Phase::Closed { Phase::Asking } else { Phase::Closed };
                    phase.set(next);
                },
                "{label}"
            }
            if open {
                Confirm { offer, asked, phase, revision }
            }
        }
    }
}

/// What will happen, and the one button that does it. A page gets its address and Copy instead:
/// this client never opens it and never fetches it.
#[component]
pub(in crate::ui) fn Confirm(
    offer: Offer,
    asked: Ask,
    phase: Signal<Phase>,
    revision: Option<Signal<u64>>,
) -> Element {
    let sentence = asked.sentence();
    let working = phase() == Phase::Working;
    let failed = match phase() {
        Phase::Failed(why) => Some(why),
        _ => None,
    };
    let cancel = "Cancel";
    let copy_label = "Copy";
    rsx! {
        div { class: "fmenu leave-ask", role: "dialog", aria_label: "Leave this list",
            div { class: "g", "Unsubscribe" }
            p { class: "say", "{sentence}" }
            if let Ask::Web { url } = &asked {
                div { class: "leave-url", {url_spans(url)} }
            }
            if let Some(why) = failed {
                p { class: "why", "{why}" }
            }
            div { class: "acts",
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{cancel}",
                    onclick: move |_| phase.set(Phase::Closed),
                    "{cancel}"
                }
                match (&asked, asked.action()) {
                    (Ask::Web { url }, _) => {
                        let url = url.clone();
                        rsx! {
                            button {
                                class: "mini primary",
                                r#type: "button",
                                aria_label: "{copy_label}",
                                onclick: move |_| copy(&url),
                                "{copy_label}"
                            }
                        }
                    }
                    (_, Some(action)) => rsx! {
                        button {
                            class: "mini primary",
                            r#type: "button",
                            aria_label: "{action}",
                            disabled: working,
                            onclick: move |_| take(offer.clone(), phase, revision),
                            if working { "Working…" } else { "{action}" }
                        }
                    },
                    (_, None) => rsx! {},
                }
            }
        }
    }
}

/// Take the way out on a blocking thread, then say what happened.
fn take(offer: Offer, mut phase: Signal<Phase>, revision: Option<Signal<u64>>) {
    if *phase.peek() == Phase::Working {
        return;
    }
    phase.set(Phase::Working);
    let store = consume_context::<Arc<SqliteStore>>();
    let desk = try_consume_context::<Desk>();
    // Spawned from a click, which is where a task is polled (F140): a future started from a
    // component body is the one thing that may never run.
    spawn(async move {
        let found = offer.found.clone();
        let blocking = store.clone();
        let done = tokio::task::spawn_blocking(move || {
            leave(&blocking, &found, chrono::Utc::now(), client)
        })
        .await;
        match done {
            Ok(Ok(outcome)) => {
                phase.set(Phase::Closed);
                landed(&store, &offer, &outcome, desk, revision);
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why)),
            Err(error) => phase.set(Phase::Failed(format!(
                "The unsubscribe stopped before it finished: {error}"
            ))),
        }
    });
}

/// The toast, and for a queued message the outbox pill, like any send.
fn landed(
    store: &SqliteStore,
    offer: &Offer,
    outcome: &Outcome,
    desk: Option<Desk>,
    revision: Option<Signal<u64>>,
) {
    let text = said(outcome, &offer.list);
    match outcome {
        Outcome::Unsubscribed { .. } => tell(
            text,
            Follow::ArchiveFrom {
                sender: offer.sender.email.clone(),
                list: offer.list.clone(),
            },
        ),
        Outcome::Queued { draft } => {
            if let Some(desk) = desk {
                show_queued(desk, store, *draft, chrono::Utc::now());
            }
            if let Some(mut revision) = revision {
                revision += 1;
            }
            tell(text, Follow::Nothing);
        }
        Outcome::Page { .. } => tell(text, Follow::Nothing),
    }
}
