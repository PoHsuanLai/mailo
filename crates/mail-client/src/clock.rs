//! When what is read by the clock changes on its own: a snoozed conversation coming back.
//!
//! The client marks stale what reads the clock ([`Topics::CLOCK`]) at that moment, on a thread
//! of its own (`mailo-clock`), so a list holding snoozed mail shows it come back on time with
//! no caller keeping a timer. The moment is found again whenever a commit touches the
//! conversations or the clock is moved.

use crate::client::Shared;
use chrono::{DateTime, Utc};
use mail_store::{SqliteStore, Topics};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// The longest the thread sleeps without looking again. A sleep is measured on a clock that
/// stops while the machine is suspended, so a snooze that ended during a suspend comes back at
/// most this long after the machine wakes.
const LONGEST: Duration = Duration::from_secs(300);

/// What a commit has to touch for the next moment to be found again.
pub(crate) const MOVES: Topics = Topics::SUMMARIES.union(Topics::CLOCK);

/// The next moment, after `now`, that what reads the clock reads differently: the earliest
/// snooze still ahead.
pub(crate) fn next(store: &SqliteStore, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    mail_core::snooze::next_wake(store, now)
}

/// How long to sleep from `now` until `next`, never longer than [`LONGEST`].
fn wait(next: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Duration {
    next.map_or(LONGEST, |at| {
        (at - now).to_std().unwrap_or(Duration::ZERO).min(LONGEST)
    })
}

/// The clock thread, until the client stops.
pub(crate) fn start(shared: Arc<Shared>) -> JoinHandle<()> {
    crate::client::spawn("mailo-clock", move || {
        while !shared.stopping() {
            let next = next(shared.store(), shared.now());
            shared.clock_sleep(wait(next, shared.now()));
            // Woken early or not: a moment passed meanwhile is marked, so a commit that came
            // with it does not hide it.
            if next.is_some_and(|at| at <= shared.now()) {
                shared.invalidate(Topics::CLOCK);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + chrono::TimeDelta::minutes(minute)
    }

    #[test]
    fn the_clock_sleeps_until_the_next_snooze_ends_within_bounds() {
        let cases = [
            ("nothing snoozed", None, LONGEST),
            ("a minute away", Some(at(1)), Duration::from_secs(60)),
            ("an hour away", Some(at(60)), LONGEST),
            ("already gone", Some(at(-1)), Duration::ZERO),
        ];
        for (name, next, want) in cases {
            assert_eq!(wait(next, at(0)), want, "{name}");
        }
    }
}
