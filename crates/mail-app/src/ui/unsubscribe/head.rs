//! Unsubscribe… in the reader's ⋯: whether the open thread offers a way out, and the popover
//! that says what it will do before it does it.

use super::super::compose::{Desk, show_queued};
use super::super::hover::{copy, url_spans};
use super::super::motion::{Follow, tell};
use super::super::press::{available, on_primary};
use super::{Ask, Bodies, Offer, ask, cached, client, leave, lookup, said};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Answers;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_core::SqliteStore;
use mail_core::unsubscribe::Outcome;
use mail_domain::ThreadId;
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

/// Whether the open thread offers a way out of its list, found for the reader's ⋯. Draws
/// nothing.
///
/// The lookup reads a stored blob, so it runs on a blocking thread and lands in `found`, or is
/// already in the cache from the last time this thread was open. Keyed by the reader on the
/// thread and its bodies, and the answer is put down with that key (`tag`), so an answer for one
/// thread can never be offered, or acted on, under another: the reader reads it back only under
/// its own key.
#[component]
pub(in crate::ui) fn Lookout(
    thread: ThreadId,
    bodies: Bodies,
    tag: String,
    found: Signal<Option<(String, Offer)>>,
) -> Element {
    let looks = crate::ui::reading::use_looks();
    let _look = use_resource(move || {
        let bodies = bodies.clone();
        let looks = looks.clone();
        let tag = tag.clone();
        let store = consume_context::<Arc<SqliteStore>>();
        let mut found = found;
        async move {
            let offer = match cached(&looks, thread, &bodies) {
                Some(offer) => offer,
                None => {
                    tokio::task::spawn_blocking(move || lookup(&looks, &store, thread, &bodies))
                        .await
                        .ok()
                        .flatten()
                }
            };
            // Only a way out this client can say something about is offered.
            let offer = offer.filter(|offer| ask(offer).is_some());
            found.set(offer.map(|offer| (tag, offer)));
        }
    });
    rsx! {}
}

/// What will happen, and the one button that does it: a quire popover under the ⋯ it was picked
/// from. A page gets its address and Copy instead: this client never opens it and never fetches
/// it.
#[component]
pub(in crate::ui) fn Confirm(
    offer: Offer,
    asked: Ask,
    phase: Signal<Phase>,
    revision: Option<Signal<u64>>,
    anchor: Anchor,
) -> Element {
    let sentence = asked.sentence();
    let working = phase() == Phase::Working;
    let failed = match phase() {
        Phase::Failed(why) => Some(why),
        _ => None,
    };
    rsx! {
            Popover {
                anchor,
                placement: Placement::new(Side::Bottom, Align::End),
                gap: Px(4.0),
                onclose: move |()| phase.set(Phase::Closed),
                common: Common {
                    extra_class: ExtraClass::parse("leave-ask").ok(),
                    aria_label: Some("Leave this list".to_owned()),
                    ..Common::default()
                },
                Label { text: "Unsubscribe", style: LabelStyle::Headline }
                Label { text: sentence }
                if let Ask::Web { url } = &asked {
                    div { class: "leave-url", {url_spans(url)} }
                }
                if let Some(why) = failed {
                    Label { text: why, role: LabelRole::Secondary }
                }
                div { class: "acts",
                    Button {
                        label: "Cancel",
                        answers: Answers::Escape,
                        onclick: on_primary(move || phase.set(Phase::Closed)),
        common: Common { aria_label: Some("Cancel".to_owned()), ..Common::default() },
    }
                    match (&asked, asked.action()) {
                        (Ask::Web { url }, _) => {
                            let url = url.clone();
                            rsx! {
                                Button {
                                    label: "Copy",
                                    answers: Answers::Return,
                                    onclick: on_primary(move || copy(&url)),
        common: Common { aria_label: Some("Copy".to_owned()), ..Common::default() },
    }
                            }
                        }
                        (_, Some(action)) => rsx! {
                            Button {
                                label: if working { "Working…".to_owned() } else { action.to_string() },
                                answers: Answers::Return,
                                availability: available(!working),
                                onclick: on_primary(move || take(offer.clone(), phase, revision)),
        common: Common { aria_label: Some(action.to_string()), ..Common::default() },
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
                "Couldn\u{2019}t unsubscribe: {error}"
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
