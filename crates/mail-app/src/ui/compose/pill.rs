//! The outbox pill: "Sending in 5 s · Undo", then the send's own state, with a ring.
//!
//! What it says is [`face`], a function of the send, the stored `SendState` and the time, so the
//! three `Retry` faces are a table in the tests rather than something to catch on screen.

use ds::components::app::send_mood::SendMood;
use ds::components::app::send_pill::{PillAction, SendPill as Pill};
use ds::motion::detail::operation::{Operation, PendingToken};
use ds::prelude::*;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::{Retry, SendState};

use super::super::menus::when_in_sentence;
use super::desk::{Desk, Outgoing, take_back_said};
use super::page::When;
use crate::ui::view::Shell;

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

/// What the pill's button offers, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Offer {
    /// Take back a send in its grace period, or one waiting in the outbox.
    Undo,
    /// Take back a send held for later. The same act as Undo, named for a send hours away.
    Cancel,
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
    if now < out.due && out.when == When::Now && matches!(state, Some(SendState::Queued)) {
        let left = (out.due - now).num_milliseconds().max(0);
        let seconds = (left + 999) / 1000;
        return Some(calm(
            format!("Sending in {seconds} s"),
            Ring::Countdown,
            Offer::Undo,
        ));
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
        SendState::Scheduled { at } => Some(calm(
            format!("Scheduled for {}", when_in_sentence(*at, now, zone)),
            Ring::Full,
            Offer::Cancel,
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
            Retry::NeedsGrant => failed(
                "Not sent · allow Mail to use the account again".to_owned(),
                Mood::Shake,
            ),
            Retry::Fatal(_) => failed(format!("Not sent: {reason}"), Mood::Fatal),
        }),
    }
}

/// Where a stopped ring rests: two sevenths of it left, as mailo's pill always drew it.
const STILL: Fraction = Fraction(702);

impl Face {
    /// How much of the grace period has gone, for quire's ring, which shows what is left. `due` is
    /// when the grace period ends, which a countdown drains towards. Every other face is drawn
    /// as a ring with nothing gone, except a stopped one, which rests where it failed.
    fn gone(&self, due: DateTime<Utc>, now: DateTime<Utc>) -> Fraction {
        match self.ring {
            Ring::Countdown => {
                let grace = super::life::GRACE.num_milliseconds().max(1);
                let left = (due - now).num_milliseconds().clamp(0, grace);
                let share = (grace - left) * 1000 / grace;
                Fraction(u16::try_from(share).unwrap_or(1000))
            }
            Ring::Spin | Ring::Full => Fraction(0),
            Ring::Still => STILL,
        }
    }
}

impl Mood {
    /// quire's pill knows two: calm, and danger. A send that will retry by itself is calm; one
    /// that needs the person, or will never go, is danger.
    fn quire(self) -> SendMood {
        match self {
            Mood::Calm | Mood::Nudge => SendMood::Calm,
            Mood::Shake | Mood::Fatal => SendMood::Fatal,
        }
    }
}

impl Offer {
    fn quire(self) -> PillAction {
        match self {
            Offer::Undo => PillAction::Undo,
            Offer::Cancel => PillAction::Cancel,
            Offer::Nothing => PillAction::Nothing,
        }
    }
}

/// The pill at the bottom of the card: quire's `SendPill`, in a box of mailo's that centres it
/// over the card rather than the window.
#[component]
pub(in crate::ui) fn SendPill(shell: Signal<Shell>) -> Element {
    let Some(desk) = try_use_context::<Desk>() else {
        return rsx! {};
    };
    let token = use_hook(PendingToken::start);
    let mut tick = use_signal(|| 0u64);
    let fetching = try_use_context::<super::super::fetching::Fetching>();
    use_future(move || async move {
        // The send this pill last asked a pass for: the outbox is drained by a pass, and nothing
        // else starts one when the grace period ends.
        let mut asked = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Some(out) = desk.outbox.peek().clone() else {
                continue;
            };
            tick += 1;
            if let Some(fetching) = fetching {
                let store = consume_context::<Arc<SqliteStore>>();
                fetching.drain_when_due(&store, out.draft, out.due, Utc::now(), &mut asked);
            }
        }
    });
    let _ = tick();
    let Some(out) = desk.outbox.read().clone() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let state = store.draft(out.draft).ok().map(|draft| draft.state);
    let now = Utc::now();
    let Some(face) = face(&out, state.as_ref(), now, &chrono::Local) else {
        return rsx! {};
    };
    let progress = face.gone(out.due, now);
    let operation = match state {
        Some(SendState::Sent { .. }) => Operation::Idle,
        _ => Operation::Running(token),
    };
    let draft = out.draft;
    rsx! {
        div { class: "send-at",
            Pill {
                text: face.text,
                progress,
                operation,
                mood: face.mood.quire(),
                action: face.offer.quire(),
                refusal: out.refused,
                // Refused, it is said on the pill and in the toast; nothing else to do here.
                onundo: move |()| {
                    let _ = take_back_said(desk, shell, draft);
                },
            }
        }
    }
}
