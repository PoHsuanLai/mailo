use super::*;
use chrono::TimeZone;
use mail_core::fetch::{First, Live, Step};
use mail_domain::Retry;

const DRAFT: DraftId = DraftId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const OTHER: DraftId = DraftId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + chrono::TimeDelta::seconds(second)
}

fn resting() -> Link {
    Link::Current {
        at: at(0),
        trouble: vec![],
        live: Live::Polling,
    }
}

fn running() -> Link {
    Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(resting()),
    }
}

/// A queued send asks for a pass when its grace period ends and not before, and a scheduled one
/// when its time comes.
#[test]
fn a_send_asks_for_a_pass_once_it_is_due() {
    let link = resting();
    let queued = SendState::Queued;
    let scheduled = SendState::Scheduled { at: at(3600) };
    // (row, its state, when it is due, seconds now, whether a pass is wanted)
    let cases: [(&str, &SendState, i64, i64, bool); 7] = [
        ("queued, just now", &queued, 5, 0, false),
        ("queued, still in grace", &queued, 5, 4, false),
        ("queued, grace ends", &queued, 5, 5, true),
        ("queued, after grace", &queued, 5, 6, true),
        ("queued, long after", &queued, 5, 600, true),
        ("scheduled, a second early", &scheduled, 3600, 3599, false),
        ("scheduled, its time", &scheduled, 3600, 3600, true),
    ];
    for (name, state, due, second, wanted) in cases {
        assert_eq!(
            wants_a_pass(DRAFT, at(due), at(second), Some(state), Some(&link), None),
            wanted,
            "{name}"
        );
    }
}

#[test]
fn only_a_send_still_waiting_in_the_outbox_asks() {
    let link = resting();
    let states = [
        ("being edited", SendState::Editing, false),
        ("on its way", SendState::Sending, false),
        (
            "sent",
            SendState::Sent {
                at: at(6),
                message: None,
            },
            false,
        ),
        (
            "failed for good",
            SendState::Failed {
                reason: "no".into(),
                retry: Retry::Fatal("no".into()),
            },
            false,
        ),
        ("queued", SendState::Queued, true),
    ];
    for (name, state, wanted) in states {
        assert_eq!(
            wants_a_pass(DRAFT, at(5), at(9), Some(&state), Some(&link), None),
            wanted,
            "{name}"
        );
    }
    assert!(
        !wants_a_pass(DRAFT, at(5), at(9), None, Some(&link), None),
        "a draft the store no longer has"
    );
}

#[test]
fn it_asks_once_per_send_and_not_while_a_pass_is_running() {
    let queued = SendState::Queued;
    assert!(
        !wants_a_pass(DRAFT, at(5), at(9), Some(&queued), Some(&running()), None),
        "a pass already running was begun before the send was due"
    );
    // The request lands the moment that pass ends: the next tick, with the link at rest.
    assert!(wants_a_pass(
        DRAFT,
        at(5),
        at(10),
        Some(&queued),
        Some(&resting()),
        None
    ));
    assert!(
        !wants_a_pass(
            DRAFT,
            at(5),
            at(10),
            Some(&queued),
            Some(&resting()),
            Some(DRAFT)
        ),
        "asked already"
    );
    assert!(
        wants_a_pass(
            DRAFT,
            at(5),
            at(10),
            Some(&queued),
            Some(&resting()),
            Some(OTHER)
        ),
        "another send's request is not this one's"
    );
    assert!(
        !wants_a_pass(DRAFT, at(5), at(10), Some(&queued), None, None),
        "an account with nothing to fetch has no pass to ask for"
    );
}
