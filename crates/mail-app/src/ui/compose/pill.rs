//! The outbox pill: "Sending in 5 s · Undo", then the send's own state, with a ring.
//!
//! What it says is [`face`], a function of the send, the stored `SendState` and the time, so the
//! three `Retry` faces are a table in the tests rather than something to catch on screen.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_domain::{Retry, SendState};
use mail_store::{SqliteStore, Store};

use super::desk::{Desk, Outgoing, undo_send};
use super::page::When;
use crate::view::Shell;

/// How long "Sent" stays before the pill leaves.
const SENT_STAYS: chrono::TimeDelta = chrono::TimeDelta::seconds(2);

/// The ring beside the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Ring {
    /// Draining over the grace period.
    Countdown,
    /// Waiting on the outbox.
    Spin,
    /// Done.
    Full,
    /// Stopped where it failed.
    Still,
}

/// Which of the retry faces the pill wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Mood {
    Calm,
    /// `Retry::After`: it will try again by itself.
    Nudge,
    /// `Retry::NeedsReauth`: the account has to be signed in again.
    Shake,
    /// `Retry::Fatal`: it will not go.
    Fatal,
}

/// Whether Undo is on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Offer {
    Undo,
    Nothing,
}

/// What the pill shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Face {
    pub text: String,
    pub ring: Ring,
    pub mood: Mood,
    pub offer: Offer,
}

/// The pill for `out`, given what the store says about it now. `None`: the pill is gone.
pub(in crate::ui) fn face<Tz: TimeZone>(
    out: &Outgoing,
    state: Option<&SendState>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Option<Face>
where
    Tz::Offset: std::fmt::Display,
{
    let calm = |text: String, ring, offer| Face {
        text,
        ring,
        mood: Mood::Calm,
        offer,
    };
    if now < out.due && matches!(state, Some(SendState::Queued)) {
        return Some(match out.when {
            When::Now => {
                let left = (out.due - now).num_milliseconds().max(0);
                let seconds = (left + 999) / 1000;
                calm(
                    format!("Sending in {seconds} s"),
                    Ring::Countdown,
                    Offer::Undo,
                )
            }
            When::Tomorrow | When::Monday => calm(
                format!(
                    "Scheduled for {}",
                    out.due.with_timezone(zone).format("%a %-d %b, %H:%M")
                ),
                Ring::Full,
                Offer::Undo,
            ),
        });
    }
    let failed = |text: String, mood| Face {
        text,
        ring: Ring::Still,
        mood,
        offer: Offer::Nothing,
    };
    match state? {
        SendState::Editing => None,
        SendState::Queued => Some(calm(
            "Waiting in the outbox".to_owned(),
            Ring::Spin,
            Offer::Undo,
        )),
        SendState::Sending => Some(calm("Sending…".to_owned(), Ring::Spin, Offer::Nothing)),
        SendState::Sent { at, .. } => {
            (now - *at < SENT_STAYS).then(|| calm("Sent".to_owned(), Ring::Full, Offer::Nothing))
        }
        SendState::Failed { retry, reason } => Some(match retry {
            Retry::Now => calm("Trying again…".to_owned(), Ring::Spin, Offer::Nothing),
            Retry::After(_) => failed("Not sent yet · will try again".to_owned(), Mood::Nudge),
            Retry::NeedsReauth => failed(
                "Not sent · sign in to the account again".to_owned(),
                Mood::Shake,
            ),
            Retry::Fatal(_) => failed(format!("Not sent: {reason}"), Mood::Fatal),
        }),
    }
}

/// The pill at the bottom of the card.
#[component]
pub(in crate::ui) fn SendPill(shell: Signal<Shell>) -> Element {
    let Some(desk) = try_use_context::<Desk>() else {
        return rsx! {};
    };
    let mut tick = use_signal(|| 0u64);
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if desk.outbox.peek().is_some() {
                tick += 1;
            }
        }
    });
    let _ = tick();
    let Some(out) = desk.outbox.read().clone() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let state = store.draft(out.draft).ok().map(|draft| draft.state);
    let Some(face) = face(&out, state.as_ref(), Utc::now(), &chrono::Local) else {
        return rsx! {};
    };
    let class = match face.mood {
        Mood::Calm => "sendpill",
        Mood::Nudge => "sendpill nudge",
        Mood::Shake => "sendpill shake",
        Mood::Fatal => "sendpill fatal",
    };
    let undo = "Undo";
    let ring = match face.ring {
        Ring::Countdown => "run countdown",
        Ring::Spin => "run spin",
        Ring::Full => "run full",
        Ring::Still => "run still",
    };
    rsx! {
        div { class: "{class}", role: "status",
            svg { view_box: "0 0 24 24",
                circle { class: "track", cx: "12", cy: "12", r: "9" }
                circle { class: "{ring}", cx: "12", cy: "12", r: "9" }
            }
            span { class: "sp-text", "{face.text}" }
            if face.offer == Offer::Undo {
                button {
                    r#type: "button",
                    aria_label: "{undo}",
                    onclick: move |_| {
                        if let Err(why) = undo_send(desk, shell) {
                            eprintln!("undo send: {why}");
                        }
                    },
                    "Undo"
                }
            }
        }
    }
}
