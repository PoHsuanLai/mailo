//! Whether an account's mail is as current as its schedule promises.
//!
//! A link that says "current" is only saying the last pass finished; it does not say when. An
//! account whose IDLE socket a NAT forgot said "Up to date" for forty hours. So freshness is
//! judged against the schedule: a polled account should have finished a pass within a few of
//! its intervals, and a pushed one within the time its watch re-issues IDLE (`IDLE_REARM`), since
//! each re-issue is followed by a look at the account. Longer than that, the account is stale,
//! whatever its link says, and the window says since when.
//!
//! When the last pass finished is kept in the store (`SqliteStore::passes`), so a window opened
//! later judges the account by its last finished pass rather than by its last ingest.

use crate::fetch::{Link, Live};
use chrono::{DateTime, TimeDelta, Utc};
use mail_runtime::IDLE_REARM;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::time::Duration;

/// How many of its intervals a polled account may miss before it is stale.
///
/// One miss is a slow pass or a laptop that just woke; three in a row is an account that is not
/// being fetched.
pub const POLLS_MISSED: u32 = 3;

/// What a pushed account is allowed past [`IDLE_REARM`] before it is stale: the pass that
/// follows a re-issue, and a little more.
pub const PUSH_SLACK: Duration = Duration::from_secs(10 * 60);

/// How current an account's mail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// No pass has finished yet: nothing to be stale against.
    Unknown,
    /// A pass finished as recently as the schedule promises.
    Current,
    /// No pass has finished since `since`, which is longer ago than the schedule allows.
    Stale { since: DateTime<Utc> },
}

/// The longest an account kept up by `live`, polled every `every`, goes between finished passes
/// before it is stale.
pub fn allowance(live: Live, every: Duration) -> Duration {
    match live {
        Live::Polling => every.saturating_mul(POLLS_MISSED),
        Live::Pushed => IDLE_REARM.saturating_add(PUSH_SLACK),
    }
}

/// How current an account is at `now`, given when its last pass `finished`.
pub fn freshness(
    finished: Option<DateTime<Utc>>,
    live: Live,
    every: Duration,
    now: DateTime<Utc>,
) -> Freshness {
    let Some(finished) = finished else {
        return Freshness::Unknown;
    };
    let allowed = TimeDelta::from_std(allowance(live, every)).unwrap_or(TimeDelta::MAX);
    match now.signed_duration_since(finished) > allowed {
        true => Freshness::Stale { since: finished },
        false => Freshness::Current,
    }
}

/// How current `link` is at `now`: judged by when its last pass finished, for a link at rest
/// that says it is current. A running pass, a wait after a failure and a stop that needs the
/// person each say more than staleness would, and are [`Freshness::Unknown`] here.
pub fn of_link(link: &Link, every: Duration, now: DateTime<Utc>) -> Freshness {
    match link {
        Link::Current { at, live, .. } => freshness(Some(*at), *live, every, now),
        _ => Freshness::Unknown,
    }
}

/// When `account`'s last pass finished, as the store recorded it, falling back to its newest
/// mailbox stamp for a store from before that record.
pub fn last_finished(store: &SqliteStore, account: AccountId) -> Option<DateTime<Utc>> {
    mail_runtime::schedule::last_synced(store, account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    const FIVE: Duration = Duration::from_secs(5 * 60);

    #[test]
    fn staleness_is_judged_against_the_schedule() {
        use Freshness::*;
        use Live::*;
        // (name, finished, live, every, now, expected)
        let cases: Vec<(
            &str,
            Option<DateTime<Utc>>,
            Live,
            Duration,
            DateTime<Utc>,
            Freshness,
        )> = vec![
            ("never finished", None, Polling, FIVE, t(600), Unknown),
            (
                "never finished, pushed",
                None,
                Pushed,
                FIVE,
                t(600),
                Unknown,
            ),
            ("polled, just now", Some(t(0)), Polling, FIVE, t(0), Current),
            (
                "polled, one interval",
                Some(t(0)),
                Polling,
                FIVE,
                t(5),
                Current,
            ),
            (
                "polled, three intervals exactly",
                Some(t(0)),
                Polling,
                FIVE,
                t(15),
                Current,
            ),
            (
                "polled, past three",
                Some(t(0)),
                Polling,
                FIVE,
                t(16),
                Stale { since: t(0) },
            ),
            (
                "polled hourly, two hours",
                Some(t(0)),
                Polling,
                Duration::from_secs(3600),
                t(120),
                Current,
            ),
            (
                "pushed, quiet half an hour",
                Some(t(0)),
                Pushed,
                FIVE,
                t(30),
                Current,
            ),
            (
                "pushed, at the allowance",
                Some(t(0)),
                Pushed,
                FIVE,
                t(35),
                Current,
            ),
            (
                "pushed, past the allowance",
                Some(t(0)),
                Pushed,
                FIVE,
                t(36),
                Stale { since: t(0) },
            ),
            (
                "pushed, forty hours",
                Some(t(0)),
                Pushed,
                FIVE,
                t(40 * 60),
                Stale { since: t(0) },
            ),
            (
                "a clock that went back",
                Some(t(10)),
                Polling,
                FIVE,
                t(0),
                Current,
            ),
        ];
        for (name, finished, live, every, now, expected) in cases {
            assert_eq!(freshness(finished, live, every, now), expected, "{name}");
        }
    }

    #[test]
    fn a_pushed_account_is_allowed_one_re_issue_and_the_look_after_it() {
        assert!(allowance(Live::Pushed, FIVE) > IDLE_REARM);
        assert!(allowance(Live::Pushed, FIVE) > crate::fetch::PUSH_SILENCE);
    }

    #[test]
    fn only_a_link_at_rest_and_current_is_judged() {
        use crate::fetch::{First, Pause, Step};
        let old = Link::Current {
            at: t(0),
            trouble: vec![],
            live: Live::Pushed,
        };
        assert_eq!(
            of_link(&old, FIVE, t(600)),
            Freshness::Stale { since: t(0) }
        );
        let others = [
            Link::Fresh,
            Link::Syncing {
                first: First::No,
                step: Step::Connecting,
                count: None,
                after: Box::new(old.clone()),
            },
            Link::Waiting {
                until: t(601),
                why: Pause::Unreachable,
                failures: 3,
                first: First::No,
            },
        ];
        for link in others {
            assert_eq!(of_link(&link, FIVE, t(600)), Freshness::Unknown, "{link:?}");
        }
    }
}
