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

#[test]
fn a_send_asks_for_a_pass_when_its_grace_period_ends_and_not_before() {
    let due = at(5);
    let queued = SendState::Queued;
    let link = resting();
    // Table: seconds since it was queued, and whether a pass is wanted.
    const CASES: &[(i64, bool)] = &[(0, false), (4, false), (5, true), (6, true), (600, true)];
    for (second, wanted) in CASES {
        assert_eq!(
            wants_a_pass(DRAFT, due, at(*second), Some(&queued), Some(&link), None),
            *wanted,
            "{second} s in"
        );
    }
}

#[test]
fn a_scheduled_send_asks_when_its_time_comes() {
    let state = SendState::Scheduled { at: at(3600) };
    let link = resting();
    for (second, wanted) in [(3599, false), (3600, true)] {
        assert_eq!(
            wants_a_pass(DRAFT, at(3600), at(second), Some(&state), Some(&link), None),
            wanted,
            "{second}"
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
