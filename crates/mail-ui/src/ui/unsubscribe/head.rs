//! Unsubscribe… in the reader's ⋯: whether the open thread offers a way out, and the alert
//! that says what it will do before it does it.

use super::super::compose::{Desk, show_queued};
use super::{Ask, Bodies, Offer, ask, cached, http, leave, lookup, said};
use dioxus::prelude::*;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Client;
use mail_client::unsubscribe::Outcome;
use mail_domain::ThreadId;
use mail_ui_kit::hover::copy;
use mail_ui_kit::motion::{Deed, Follow, tell};

/// Where the alert is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Phase {
    Closed,
    Asking,
    /// The way out is being taken; a second press of its button does nothing, so it cannot send twice.
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
        let client = mail_ui_kit::live::client();
        let mut found = found;
        async move {
            let offer = match cached(&looks, thread, &bodies) {
                Some(offer) => offer,
                None => {
                    tokio::task::spawn_blocking(move || lookup(&looks, &client, thread, &bodies))
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

/// What will happen, and the one button that does it: a quire alert. A page gets its address and
/// Copy instead: this client never opens it and never fetches it. While the way out is being
/// taken the alert stays up and says "Working…", and a failure is said in its message.
#[component]
pub(in crate::ui) fn Confirm(offer: Offer, asked: Ask, phase: Signal<Phase>) -> Element {
    let message = match (phase(), &asked) {
        (Phase::Working, _) => "Working\u{2026}".to_owned(),
        (Phase::Failed(why), _) => why,
        (_, Ask::Web { url }) => format!("{} {url}", asked.sentence()),
        _ => asked.sentence(),
    };
    let close = EventHandler::new(move |()| phase.set(Phase::Closed));
    let answer = match (&asked, asked.action()) {
        (Ask::Web { url }, _) => {
            let url = url.clone();
            Some(("Copy", EventHandler::new(move |()| copy(&url))))
        }
        (_, Some(action)) => Some((
            action,
            EventHandler::new(move |()| take(offer.clone(), phase)),
        )),
        (_, None) => None,
    };
    let mut buttons: Vec<AlertButton> = answer
        .into_iter()
        .map(|(label, onpress)| {
            AlertButton::new(label, AlertRole::Normal, onpress).with_common(Common {
                aria_label: Some(label.to_owned()),
                ..Common::default()
            })
        })
        .collect();
    buttons.push(AlertButton::new(
        mail_ui_model::copy::controls::CANCEL,
        AlertRole::Cancel,
        close,
    ));
    rsx! {
        Alert {
            title: "Unsubscribe",
            message: Some(TextLine::from(message)),
            style: AlertStyle::Informational,
            buttons,
            common: Common {
                aria_label: Some("Leave this list".to_owned()),
                ..Common::default()
            },
        }
    }
}

/// Take the way out on a blocking thread, then say what happened.
fn take(offer: Offer, mut phase: Signal<Phase>) {
    if *phase.peek() == Phase::Working {
        return;
    }
    phase.set(Phase::Working);
    let client = mail_ui_kit::live::client();
    let desk = try_consume_context::<Desk>();
    // Spawned from a click, which is where a task is polled: a future started from a
    // component body is the one thing that may never run.
    spawn(async move {
        let found = offer.found.clone();
        let blocking = client.clone();
        let done =
            tokio::task::spawn_blocking(move || leave(&blocking, &found, chrono::Utc::now(), http))
                .await;
        match done {
            Ok(Ok(outcome)) => {
                phase.set(Phase::Closed);
                landed(&client, &offer, &outcome, desk);
            }
            Ok(Err(why)) => phase.set(Phase::Failed(why)),
            Err(error) => phase.set(Phase::Failed(format!(
                "Couldn\u{2019}t unsubscribe: {error}"
            ))),
        }
    });
}

/// The toast, and for a queued message the outbox pill, like any send.
fn landed(client: &Client, offer: &Offer, outcome: &Outcome, desk: Option<Desk>) {
    let text = said(outcome, &offer.list);
    match outcome {
        Outcome::Unsubscribed { .. } => tell(text, {
            let (sender, list) = (offer.sender.email.clone(), offer.list.clone());
            Follow::Do(Deed::new(
                mail_ui_model::copy::controls::ARCHIVE_ALL,
                move |client, shell| {
                    super::archive_list(client, shell, &sender, &list);
                },
            ))
        }),
        Outcome::Queued { draft } => {
            if let Some(desk) = desk {
                show_queued(desk, client, *draft, chrono::Utc::now());
            }
            tell(text, Follow::Nothing);
        }
        Outcome::Page { .. } => tell(text, Follow::Nothing),
    }
}
