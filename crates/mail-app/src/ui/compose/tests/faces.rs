//! What the outbox pill says, for every state a send can be in.

use super::super::desk::Outgoing;
use super::super::page::When;
use super::super::pill::{Face, Mood, Offer, Ring, face};
use super::*;

/// A name, the send, its stored state, how long after sending, and the face wanted.
type Case = (&'static str, When, SendState, i64, Option<Want>);
type Want = (&'static str, Ring, Mood, Offer);

fn outgoing(when: When, due: DateTime<Utc>) -> Outgoing {
    let page = page_of("hello");
    Outgoing {
        draft: page.draft,
        due,
        when,
        page,
    }
}

#[test]
fn the_pill_wears_the_face_of_the_send() {
    let due = at(0) + chrono::TimeDelta::seconds(5);
    let failed = |retry: Retry| SendState::Failed {
        reason: "the server said no".to_owned(),
        retry,
    };
    let sent = SendState::Sent {
        at: at(0) + chrono::TimeDelta::seconds(6),
        message: None,
    };
    let cases: Vec<Case> = vec![
        (
            "counting down",
            When::Now,
            SendState::Queued,
            0,
            Some(("Sending in 5 s", Ring::Countdown, Mood::Calm, Offer::Undo)),
        ),
        (
            "one second later",
            When::Now,
            SendState::Queued,
            1100,
            Some(("Sending in 4 s", Ring::Countdown, Mood::Calm, Offer::Undo)),
        ),
        (
            "past the grace, still queued",
            When::Now,
            SendState::Queued,
            6000,
            Some(("Waiting in the outbox", Ring::Spin, Mood::Calm, Offer::Undo)),
        ),
        (
            "on the wire",
            When::Now,
            SendState::Sending,
            6000,
            Some(("Sending…", Ring::Spin, Mood::Calm, Offer::Nothing)),
        ),
        (
            "sent",
            When::Now,
            sent.clone(),
            6500,
            Some(("Sent", Ring::Full, Mood::Calm, Offer::Nothing)),
        ),
        ("sent a while ago", When::Now, sent, 60_000, None),
        (
            "retry after",
            When::Now,
            failed(Retry::After(std::time::Duration::from_secs(60))),
            7000,
            Some((
                "Not sent yet · will try again",
                Ring::Still,
                Mood::Nudge,
                Offer::Nothing,
            )),
        ),
        (
            "needs reauth",
            When::Now,
            failed(Retry::NeedsReauth),
            7000,
            Some((
                "Not sent · sign in to the account again",
                Ring::Still,
                Mood::Shake,
                Offer::Nothing,
            )),
        ),
        (
            "fatal",
            When::Now,
            failed(Retry::Fatal("rejected".to_owned())),
            7000,
            Some((
                "Not sent: the server said no",
                Ring::Still,
                Mood::Fatal,
                Offer::Nothing,
            )),
        ),
        ("undone", When::Now, SendState::Editing, 1000, None),
    ];
    for (name, when, state, after_ms, want) in cases {
        let now = at(0) + chrono::TimeDelta::milliseconds(after_ms);
        let got = face(&outgoing(when, due), Some(&state), now, &Utc);
        let want = want.map(|(text, ring, mood, offer)| Face {
            text: text.to_owned(),
            ring,
            mood,
            offer,
        });
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn a_scheduled_send_reads_scheduled_for() {
    let due = When::Tomorrow
        .due(at(0), &Utc)
        .unwrap_or_else(|| panic!("tomorrow exists"));
    let got = face(
        &outgoing(When::Tomorrow, due),
        Some(&SendState::Queued),
        at(0),
        &Utc,
    );
    assert_eq!(
        got.map(|face| (face.text, face.offer)),
        Some(("Scheduled for Thu 24 Sep, 08:00".to_owned(), Offer::Undo))
    );
}
