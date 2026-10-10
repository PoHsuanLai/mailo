//! The transition table, row by row.

use super::*;
use chrono::TimeZone;

const EVERY: Duration = Duration::from_secs(300);

fn t(minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, minute, second)
        .unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn current(at: DateTime<Utc>, live: Live) -> Link {
    Link::Current {
        at,
        trouble: vec![],
        live,
    }
}

fn waiting(until: DateTime<Utc>, failures: u32) -> Link {
    Link::Waiting {
        until,
        why: Pause::Unreachable,
        failures,
        first: First::No,
    }
}

fn syncing(first: First, after: Link) -> Link {
    Link::Syncing {
        first,
        step: Step::Connecting,
        count: None,
        after: Box::new(after),
    }
}

fn broken() -> Link {
    Link::Broken {
        why: "no".into(),
        first: First::No,
    }
}

fn signin() -> Link {
    Link::NeedsSignIn {
        why: "rejected".into(),
        first: First::No,
    }
}

fn failed(retry: Retry, pause: Pause) -> Event {
    Event::Failed {
        retry,
        why: "why".into(),
        pause,
    }
}

fn go(link: &Link, event: Event, now: DateTime<Utc>) -> (Link, Vec<Effect>) {
    step(link, event, now, EVERY)
}

#[test]
fn start_from_resting_states_begins_a_pass() {
    let now = t(10, 0);
    let cases = [
        (Link::Fresh, First::Yes),
        (current(t(0, 0), Live::Polling), First::No),
        (waiting(t(20, 0), 2), First::No),
    ];
    for (from, first) in cases {
        for trigger in [
            Trigger::Poll,
            Trigger::Push,
            Trigger::Manual,
            Trigger::FolderOpen,
        ] {
            let (to, fx) = go(&from, Event::Start(trigger), now);
            assert_eq!(to, syncing(first, from.clone()), "{from:?} {trigger:?}");
            assert_eq!(fx, vec![Effect::RunPass]);
        }
    }
}

#[test]
fn a_second_start_while_running_is_the_same_request() {
    let running = syncing(First::No, Link::Fresh);
    assert_eq!(
        go(&running, Event::Start(Trigger::Manual), t(1, 0)),
        (running, vec![])
    );
}

#[test]
fn stepping_updates_step_and_count() {
    let running = syncing(First::Yes, Link::Fresh);
    let count = Some(Count { done: 3, of: 10 });
    let (to, fx) = go(&running, Event::Stepped(Step::Bodies, count), t(1, 0));
    assert_eq!(
        to,
        Link::Syncing {
            first: First::Yes,
            step: Step::Bodies,
            count,
            after: Box::new(Link::Fresh)
        }
    );
    assert!(fx.is_empty());
}

#[test]
fn finishing_polls_again_after_the_interval() {
    let now = t(3, 0);
    let trouble = vec![Trouble {
        mailbox: Some("A".into()),
        retry: Retry::Now,
        why: "x".into(),
    }];
    let (to, fx) = go(
        &syncing(First::Yes, Link::Fresh),
        Event::Finished {
            trouble: trouble.clone(),
        },
        now,
    );
    assert_eq!(
        to,
        Link::Current {
            at: now,
            trouble,
            live: Live::Polling
        }
    );
    assert_eq!(fx, vec![Effect::WakeAt(t(8, 0))]);
}

#[test]
fn finishing_carries_push_and_sets_no_timer() {
    let now = t(3, 0);
    let running = syncing(First::No, current(t(0, 0), Live::Pushed));
    let (to, fx) = go(&running, Event::Finished { trouble: vec![] }, now);
    assert_eq!(to, current(now, Live::Pushed));
    assert!(fx.is_empty());
}

#[test]
fn finishing_resets_failures() {
    let running = syncing(First::No, waiting(t(0, 0), 6));
    let (to, _) = go(&running, Event::Finished { trouble: vec![] }, t(1, 0));
    assert_eq!(to, current(t(1, 0), Live::Polling));
    let (again, _) = go(&to, Event::Start(Trigger::Poll), t(1, 0));
    let (to, _) = go(
        &again,
        failed(Retry::After(secs(0)), Pause::Unreachable),
        t(2, 0),
    );
    assert_eq!(to, waiting(t(7, 0), 1));
}

#[test]
fn failing_now_waits_a_few_seconds() {
    let (to, fx) = go(
        &syncing(First::No, current(t(0, 0), Live::Polling)),
        failed(Retry::Now, Pause::Unreachable),
        t(1, 0),
    );
    assert_eq!(to, waiting(t(1, 5), 1));
    assert_eq!(fx, vec![Effect::WakeAt(t(1, 5))]);
}

#[test]
fn failing_after_doubles_to_the_ceiling() {
    // Interval 5 minutes: 5, 10, 20, then the 30 minute ceiling, and it stays there.
    let mut link = current(t(0, 0), Live::Polling);
    let mut now = t(0, 0);
    let mut waits = vec![];
    for _ in 0..6 {
        let (running, _) = go(&link, Event::Start(Trigger::Poll), now);
        let (next, _) = go(
            &running,
            failed(Retry::After(secs(1)), Pause::Unreachable),
            now,
        );
        let Link::Waiting { until, .. } = next else {
            panic!("{next:?}")
        };
        waits.push((until - now).num_minutes());
        now = until;
        link = next;
    }
    assert_eq!(waits, vec![5, 10, 20, 30, 30, 30]);
}

#[test]
fn the_named_wait_is_a_floor() {
    let (to, _) = go(
        &syncing(First::No, Link::Fresh),
        failed(Retry::After(secs(900)), Pause::ServerBusy),
        t(0, 0),
    );
    assert_eq!(
        to,
        Link::Waiting {
            until: t(15, 0),
            why: Pause::ServerBusy,
            failures: 1,
            first: First::No
        }
    );
}

#[test]
fn an_ordinary_named_wait_is_capped_but_a_throttle_is_honoured_in_full() {
    let hours = Retry::After(secs(3 * 3600));
    let running = syncing(First::No, Link::Fresh);
    let (to, _) = go(&running, failed(hours.clone(), Pause::Unreachable), t(0, 0));
    assert_eq!(
        to,
        Link::Waiting {
            until: t(30, 0),
            why: Pause::Unreachable,
            failures: 1,
            first: First::No
        }
    );
    let (to, fx) = go(&running, failed(hours, Pause::Throttled), t(0, 0));
    let until = t(0, 0) + TimeDelta::hours(3);
    assert_eq!(
        to,
        Link::Waiting {
            until,
            why: Pause::Throttled,
            failures: 1,
            first: First::No
        }
    );
    assert_eq!(fx, vec![Effect::WakeAt(until)]);
}

#[test]
fn a_huge_wait_does_not_overflow() {
    let (to, _) = go(
        &syncing(First::No, Link::Fresh),
        failed(Retry::After(Duration::MAX), Pause::Throttled),
        t(0, 0),
    );
    assert!(matches!(to, Link::Waiting { .. }));
}

#[test]
fn rejection_and_fatal_stop_without_a_wake() {
    let running = syncing(First::No, Link::Fresh);
    let (to, fx) = go(
        &running,
        Event::Failed {
            retry: Retry::NeedsReauth,
            why: "bad".into(),
            pause: Pause::Unreachable,
        },
        t(0, 0),
    );
    assert_eq!(
        (to, fx),
        (
            Link::NeedsSignIn {
                why: "bad".into(),
                first: First::No
            },
            vec![]
        )
    );
    // A grant withdrawn is its own stop: allowing Mail again, not signing in, starts it again.
    let (to, fx) = go(
        &running,
        failed(Retry::NeedsGrant, Pause::Unreachable),
        t(0, 0),
    );
    assert!(matches!(to, Link::NeedsAllow { .. }), "{to:?}");
    assert!(fx.is_empty());
    assert!(!to.may_start(Trigger::Poll));
    let (again, _) = go(&to, Event::SignedIn, t(0, 1));
    assert!(again.is_busy(), "{again:?}");
    let (to, fx) = go(
        &running,
        failed(Retry::Fatal("gone".into()), Pause::Unreachable),
        t(0, 0),
    );
    assert_eq!(
        (to, fx),
        (
            Link::Broken {
                why: "gone".into(),
                first: First::No
            },
            vec![]
        )
    );
}

#[test]
fn cancel_returns_to_where_it_began() {
    for from in [
        Link::Fresh,
        current(t(0, 0), Live::Pushed),
        waiting(t(9, 0), 3),
    ] {
        let (to, fx) = go(&syncing(First::No, from.clone()), Event::Cancel, t(1, 0));
        assert_eq!((to, fx), (from, vec![Effect::CancelPass]));
    }
}

#[test]
fn tick_polls_a_current_link_once_the_interval_has_passed() {
    let link = current(t(0, 0), Live::Polling);
    assert_eq!(go(&link, Event::Tick, t(4, 59)), (link.clone(), vec![]));
    let (to, fx) = go(&link, Event::Tick, t(5, 0));
    assert_eq!((to, fx), (syncing(First::No, link), vec![Effect::RunPass]));
}

#[test]
fn tick_never_polls_a_pushed_link() {
    let link = current(t(0, 0), Live::Pushed);
    assert_eq!(go(&link, Event::Tick, t(59, 0)), (link, vec![]));
}

#[test]
fn tick_retries_a_waiting_link_when_due() {
    let link = waiting(t(10, 0), 2);
    assert_eq!(go(&link, Event::Tick, t(9, 59)), (link.clone(), vec![]));
    let (to, fx) = go(&link, Event::Tick, t(10, 0));
    assert_eq!((to, fx), (syncing(First::No, link), vec![Effect::RunPass]));
}

#[test]
fn signing_in_starts_a_pass() {
    let (to, fx) = go(&signin(), Event::SignedIn, t(0, 0));
    assert_eq!(
        (to, fx),
        (syncing(First::No, signin()), vec![Effect::RunPass])
    );
}

/// What a link at `first` rests as, for each way a first pass can fail.
fn failures_from_fresh() -> Vec<(&'static str, Event)> {
    vec![
        (
            "unreachable",
            failed(Retry::After(secs(5)), Pause::Unreachable),
        ),
        ("dropped", failed(Retry::Now, Pause::Unreachable)),
        ("rejected", failed(Retry::NeedsReauth, Pause::Unreachable)),
        (
            "fatal",
            failed(Retry::Fatal("no".into()), Pause::ServerBusy),
        ),
    ]
}

#[test]
fn a_retry_after_a_failed_first_pass_is_still_the_first() {
    // The list says "Empty" if it is not: nothing has ever been fetched, so a skeleton is the
    // truth. Every way the first pass can fail, then every way back into a pass.
    for (name, event) in failures_from_fresh() {
        let (resting, _) = go(&syncing(First::Yes, Link::Fresh), event, t(0, 0));
        assert_eq!(
            resting.first(),
            First::Yes,
            "{name}: failing forgot it was the first"
        );
        let start = match resting {
            Link::NeedsSignIn { .. } => Event::SignedIn,
            _ => Event::Start(Trigger::Manual),
        };
        let (again, fx) = go(&resting, start, t(10, 0));
        assert_eq!(fx, vec![Effect::RunPass], "{name}");
        assert!(
            matches!(
                again,
                Link::Syncing {
                    first: First::Yes,
                    ..
                }
            ),
            "{name}: the retry is not the first pass: {again:?}"
        );
    }
}

#[test]
fn a_retry_that_waits_twice_is_still_the_first_until_one_finishes() {
    let (first, _) = go(
        &syncing(First::Yes, Link::Fresh),
        failed(Retry::After(secs(5)), Pause::Unreachable),
        t(0, 0),
    );
    let (second, _) = go(&first, Event::Tick, t(10, 0));
    let (again, _) = go(
        &second,
        failed(Retry::After(secs(5)), Pause::Unreachable),
        t(10, 1),
    );
    assert_eq!(again.first(), First::Yes);
    let (running, _) = go(&again, Event::Tick, t(40, 0));
    let (done, _) = go(&running, Event::Finished { trouble: vec![] }, t(41, 0));
    assert_eq!(done.first(), First::No, "a finished pass ends it");
}

#[test]
fn an_account_that_has_been_fetched_never_goes_back_to_first() {
    let (resting, _) = go(
        &syncing(First::No, current(t(0, 0), Live::Polling)),
        failed(Retry::After(secs(5)), Pause::Unreachable),
        t(1, 0),
    );
    assert_eq!(resting.first(), First::No);
    let (again, _) = go(&resting, Event::Start(Trigger::Manual), t(2, 0));
    assert!(matches!(
        again,
        Link::Syncing {
            first: First::No,
            ..
        }
    ));
}

#[test]
fn cancelling_a_first_pass_returns_to_fresh() {
    let (to, _) = go(&syncing(First::Yes, Link::Fresh), Event::Cancel, t(0, 0));
    assert_eq!((to.first(), to), (First::Yes, Link::Fresh));
}

#[test]
fn a_broken_link_starts_only_when_a_person_asks() {
    let link = broken();
    for trigger in [Trigger::Poll, Trigger::Push, Trigger::FolderOpen] {
        assert_eq!(
            go(&link, Event::Start(trigger), t(0, 0)),
            (link.clone(), vec![])
        );
    }
    let (to, fx) = go(&link, Event::Start(Trigger::Manual), t(0, 0));
    assert_eq!((to, fx), (syncing(First::No, link), vec![Effect::RunPass]));
}

#[test]
fn needing_sign_in_ignores_everything_but_signing_in() {
    let link = signin();
    let events = [
        Event::Start(Trigger::Manual),
        Event::Tick,
        Event::Cancel,
        Event::Finished { trouble: vec![] },
        Event::Stepped(Step::Flags, None),
    ];
    for event in events {
        assert_eq!(
            go(&link, event.clone(), t(0, 0)),
            (link.clone(), vec![]),
            "{event:?}"
        );
    }
}

#[test]
fn live_changes_update_a_current_link() {
    let link = current(t(0, 0), Live::Polling);
    assert_eq!(
        go(&link, Event::Live(Live::Pushed), t(1, 0)),
        (current(t(0, 0), Live::Pushed), vec![])
    );
    let (to, fx) = go(
        &current(t(0, 0), Live::Pushed),
        Event::Live(Live::Polling),
        t(1, 0),
    );
    assert_eq!((to, fx), (link.clone(), vec![Effect::WakeAt(t(5, 0))]));
    assert_eq!(
        go(&link, Event::Live(Live::Polling), t(1, 0)),
        (link, vec![])
    );
}

#[test]
fn live_while_running_is_kept_for_the_finish() {
    let running = syncing(First::No, current(t(0, 0), Live::Polling));
    let (running, fx) = go(&running, Event::Live(Live::Pushed), t(1, 0));
    assert!(fx.is_empty());
    let (to, _) = go(&running, Event::Finished { trouble: vec![] }, t(2, 0));
    assert_eq!(to, current(t(2, 0), Live::Pushed));
}

#[test]
fn events_that_do_not_apply_do_nothing() {
    let rest = [
        (Link::Fresh, Event::Tick),
        (Link::Fresh, Event::Cancel),
        (Link::Fresh, Event::Finished { trouble: vec![] }),
        (current(t(0, 0), Live::Polling), Event::Cancel),
        (current(t(0, 0), Live::Polling), Event::SignedIn),
        (waiting(t(9, 0), 1), Event::SignedIn),
        (waiting(t(9, 0), 1), Event::Live(Live::Pushed)),
        (broken(), Event::Tick),
        (broken(), Event::SignedIn),
    ];
    for (link, event) in rest {
        assert_eq!(
            go(&link, event.clone(), t(1, 0)),
            (link.clone(), vec![]),
            "{link:?} {event:?}"
        );
    }
}

#[test]
fn helpers_say_what_the_link_is_doing() {
    let running = syncing(First::No, current(t(0, 0), Live::Polling));
    assert!(running.is_busy());
    assert_eq!(running.resting(), &current(t(0, 0), Live::Polling));
    assert!(!Link::Fresh.is_busy());
    assert!(Link::Fresh.may_start(Trigger::Poll));
    assert!(!running.may_start(Trigger::Manual));
    assert!(!broken().may_start(Trigger::Poll));
    assert!(broken().may_start(Trigger::Manual));
    assert!(!signin().may_start(Trigger::Manual));
}

#[test]
fn backoff_is_table_driven() {
    const CASES: &[(u32, u64)] = &[
        (0, 300),
        (1, 300),
        (2, 600),
        (3, 1200),
        (4, 1800),
        (17, 1800),
        (u32::MAX, 1800),
    ];
    for (failures, want) in CASES {
        assert_eq!(
            backoff(*failures, EVERY),
            secs(*want),
            "{failures} failures"
        );
    }
    // An interval longer than the ceiling is not shortened by it.
    assert_eq!(backoff(1, secs(3600)), secs(3600));
}
