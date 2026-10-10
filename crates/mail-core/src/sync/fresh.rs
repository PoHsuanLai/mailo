//! Whether an account's mail is as current as its schedule promises.
//!
//! A link that says "current" is only saying the last pass finished; it does not say when. An
//! account whose IDLE socket a NAT forgot said "Up to date" for forty hours. So freshness is
//! judged against the schedule: a polled account should have finished a pass within a few of
//! its intervals, and one the server pushes to within the time a watch re-issues IDLE
//! (`IDLE_REARM`), since each re-issue is followed by a look at the account. Longer than that,
//! the account is stale, whatever its link says, and the window says since when.
//!
//! When a pass last finished is kept in the store (`SqliteStore::passes`) by whichever process
//! ran it, so a window that leaves fetching to a `mailo watch` judges the account by the watch's
//! passes as well as its own, and one opened later by the last finished pass rather than by the
//! last ingest. The allowance follows the server ([`WatchMode`]), not who holds the connection:
//! the same account is pushed by a watch and polled by a window beside it.

use crate::fetch::Link;
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::WatchMode;
use mail_runtime::IDLE_REARM;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::time::Duration;

/// How many of its intervals a polled account may miss before it is stale.
///
/// One miss is a slow pass or a laptop that just woke; three in a row is an account that is not
/// being fetched.
pub const POLLS_MISSED: u32 = 3;

/// What an account the server pushes to is allowed past [`IDLE_REARM`] before it is stale: the
/// pass that follows a re-issue, and a little more.
pub const PUSH_SLACK: Duration = Duration::from_secs(10 * 60);

/// How current an account's mail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// No pass has finished yet, or the link says something more pressing than staleness.
    Unknown,
    /// A pass finished as recently as the schedule promises.
    Current,
    /// No pass has finished since `since`, which is longer ago than the schedule allows.
    Stale { since: DateTime<Utc> },
}

/// What an account's freshness is judged by besides its link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// When the store last recorded a finished pass of the account, by any process.
    pub recorded: Option<DateTime<Utc>>,
    /// The longest the account goes between finished passes before it is stale.
    pub allowed: Duration,
}

/// The longest an account whose server says `watch` goes between finished passes before it is
/// stale.
pub fn allowance(watch: &WatchMode) -> Duration {
    match watch {
        WatchMode::Poll { every } => every.saturating_mul(POLLS_MISSED),
        WatchMode::Idle => IDLE_REARM.saturating_add(PUSH_SLACK),
    }
}

impl Pace {
    /// An account kept up as `watch` says, with no finished pass recorded.
    pub fn of_watch(watch: &WatchMode) -> Pace {
        Pace {
            recorded: None,
            allowed: allowance(watch),
        }
    }

    /// `account`'s pace as the store has it: its recorded finish, and the allowance its stored
    /// capabilities give. An account with none yet is taken to poll at the default interval.
    pub fn of(store: &SqliteStore, account: AccountId) -> Pace {
        let recorded = store
            .passes(account.clone())
            .ok()
            .and_then(|passes| passes.finished);
        let watch = store.account_caps(account).ok().flatten().map_or(
            WatchMode::Poll {
                every: super::due::IDLE,
            },
            |caps| caps.watch,
        );
        Pace {
            recorded,
            ..Pace::of_watch(&watch)
        }
    }

    /// When `link` last finished a pass: the later of what it holds and what the store
    /// recorded. `None` for a link that is not at rest and current.
    pub fn finished(&self, link: &Link) -> Option<DateTime<Utc>> {
        match link {
            Link::Current { at, .. } => {
                Some(self.recorded.map_or(*at, |recorded| recorded.max(*at)))
            }
            _ => None,
        }
    }
}

/// How current an account is at `now`, given when its last pass `finished` and what it is
/// `allowed`.
pub fn freshness(
    finished: Option<DateTime<Utc>>,
    allowed: Duration,
    now: DateTime<Utc>,
) -> Freshness {
    let Some(finished) = finished else {
        return Freshness::Unknown;
    };
    let allowed = TimeDelta::from_std(allowed).unwrap_or(TimeDelta::MAX);
    match now.signed_duration_since(finished) > allowed {
        true => Freshness::Stale { since: finished },
        false => Freshness::Current,
    }
}

/// How current `link` is at `now`, judged for a link at rest that says it is current. A running
/// pass, a wait after a failure and a stop that needs the person each say more than staleness
/// would, and are [`Freshness::Unknown`] here.
pub fn of_link(link: &Link, pace: Pace, now: DateTime<Utc>) -> Freshness {
    freshness(pace.finished(link), pace.allowed, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Live;
    use chrono::TimeZone;

    fn t(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    const FIVE: Duration = Duration::from_secs(5 * 60);

    fn polled(every: Duration) -> Duration {
        allowance(&WatchMode::Poll { every })
    }

    fn pushed() -> Duration {
        allowance(&WatchMode::Idle)
    }

    #[test]
    fn staleness_is_judged_against_the_schedule() {
        use Freshness::*;
        // (name, finished, allowed, now, expected)
        type Row = (
            &'static str,
            Option<DateTime<Utc>>,
            Duration,
            DateTime<Utc>,
            Freshness,
        );
        let rows: Vec<Row> = vec![
            ("never finished", None, polled(FIVE), t(600), Unknown),
            ("never finished, pushed", None, pushed(), t(600), Unknown),
            ("polled, just now", Some(t(0)), polled(FIVE), t(0), Current),
            (
                "polled, one interval",
                Some(t(0)),
                polled(FIVE),
                t(5),
                Current,
            ),
            (
                "polled, three intervals",
                Some(t(0)),
                polled(FIVE),
                t(15),
                Current,
            ),
            (
                "polled, past three",
                Some(t(0)),
                polled(FIVE),
                t(16),
                Stale { since: t(0) },
            ),
            (
                "polled hourly, two hours",
                Some(t(0)),
                polled(Duration::from_secs(3600)),
                t(120),
                Current,
            ),
            (
                "pushed, quiet half an hour",
                Some(t(0)),
                pushed(),
                t(30),
                Current,
            ),
            (
                "pushed, at the allowance",
                Some(t(0)),
                pushed(),
                t(35),
                Current,
            ),
            (
                "pushed, past it",
                Some(t(0)),
                pushed(),
                t(36),
                Stale { since: t(0) },
            ),
            (
                "pushed, forty hours",
                Some(t(0)),
                pushed(),
                t(2400),
                Stale { since: t(0) },
            ),
            (
                "a clock that went back",
                Some(t(10)),
                polled(FIVE),
                t(0),
                Current,
            ),
        ];
        for (name, finished, allowed, now, expected) in rows {
            assert_eq!(freshness(finished, allowed, now), expected, "{name}");
        }
    }

    #[test]
    fn a_pushed_account_is_allowed_one_re_issue_and_the_look_after_it() {
        assert!(pushed() > IDLE_REARM);
        assert!(pushed() > crate::fetch::PUSH_SILENCE);
    }

    /// A window that leaves fetching to a watch never sees the watch's passes in its own link;
    /// the store's record of them counts, and the later of the two is the one judged.
    #[test]
    fn a_pass_recorded_by_another_process_counts() {
        let link = Link::Current {
            at: t(0),
            trouble: vec![],
            live: Live::Polling,
        };
        let alone = Pace::of_watch(&WatchMode::Idle);
        assert_eq!(
            of_link(&link, alone, t(60)),
            Freshness::Stale { since: t(0) }
        );
        let told = Pace {
            recorded: Some(t(40)),
            ..alone
        };
        assert_eq!(of_link(&link, told, t(60)), Freshness::Current);
        assert_eq!(told.finished(&link), Some(t(40)));
        // An older record does not make the link older than it is.
        let older = Pace {
            recorded: Some(t(-60)),
            ..alone
        };
        assert_eq!(older.finished(&link), Some(t(0)));
    }

    #[test]
    fn only_a_link_at_rest_and_current_is_judged() {
        use crate::fetch::{First, Pause, Step};
        let pace = Pace::of_watch(&WatchMode::Idle);
        let old = Link::Current {
            at: t(0),
            trouble: vec![],
            live: Live::Pushed,
        };
        assert_eq!(
            of_link(&old, pace, t(600)),
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
            assert_eq!(of_link(&link, pace, t(600)), Freshness::Unknown, "{link:?}");
        }
    }
}
