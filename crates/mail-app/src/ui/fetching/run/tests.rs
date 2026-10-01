use super::*;
use chrono::TimeZone;
use mail_core::fetch::{First, Live, Step};

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn current() -> Link {
    Link::Current {
        at: at(0),
        trouble: vec![],
        live: Live::Polling,
    }
}

#[test]
fn the_spinner_runs_only_while_a_pass_does() {
    let token = Operation::Running(PendingToken::start());
    let syncing = Link::Syncing {
        first: First::No,
        step: Step::Flags,
        count: None,
        after: Box::new(current()),
    };
    let resting = [
        current(),
        Link::Fresh,
        Link::Waiting {
            until: at(5),
            why: mail_core::fetch::Pause::Unreachable,
            failures: 1,
            first: First::No,
        },
        Link::NeedsSignIn {
            why: "x".into(),
            first: First::No,
        },
        Link::Broken {
            why: "x".into(),
            first: First::No,
        },
    ];
    assert_eq!(settled(token, &syncing), token, "it keeps its own token");
    for link in resting {
        assert_eq!(settled(token, &link), Operation::Idle, "{link:?}");
    }
    assert_eq!(settled(Operation::Idle, &current()), Operation::Idle);
}

#[test]
fn a_wake_sleeps_until_its_time_and_a_moment_over() {
    const CASES: &[(i64, Duration)] = &[(0, SLACK), (-30, SLACK)];
    for (offset, expected) in CASES {
        assert_eq!(delay(at(*offset), at(0)), *expected, "{offset} s from now");
    }
    assert_eq!(delay(at(300), at(0)), Duration::from_secs(300) + SLACK);
}

#[test]
fn a_wake_at_the_end_of_time_is_capped_not_overflowed() {
    assert_eq!(delay(DateTime::<Utc>::MAX_UTC, at(0)), FAR + SLACK);
}
