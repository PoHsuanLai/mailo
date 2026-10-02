//! A folder fetched because someone opened it. Keyed by (account, mailbox) by the caller.

use chrono::{DateTime, TimeDelta, Utc};

/// How long an opened folder is left alone before opening it fetches again.
///
/// The same sixty seconds `ui::folder_open` has always used: clicking between two folders must
/// not become a fetch per click.
pub const AGAIN: TimeDelta = TimeDelta::seconds(60);

/// Where one folder's on-demand fetch stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FolderFetch {
    /// Not fetched since the window opened.
    #[default]
    Unfetched,
    /// A fetch is running.
    Fetching,
    /// Fetched at this time.
    Fetched { at: DateTime<Utc> },
    /// The server said no, and what it said.
    Refused(String),
}

/// What happened to the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderEvent {
    /// Someone opened it.
    Open,
    /// The fetch finished.
    Done,
    /// The fetch was refused.
    Refused(String),
    /// Sync was pressed: every folder fetches again on its next opening.
    Forget,
}

/// What the caller must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderEffect {
    Fetch,
}

impl FolderFetch {
    /// What `event` does to the folder at `now`.
    pub fn step(
        &self,
        event: FolderEvent,
        now: DateTime<Utc>,
    ) -> (FolderFetch, Option<FolderEffect>) {
        match (self, event) {
            (FolderFetch::Fetched { at }, FolderEvent::Open) if *at <= now && now - *at < AGAIN => {
                (self.clone(), None)
            }
            (FolderFetch::Fetching, FolderEvent::Open) => (FolderFetch::Fetching, None),
            (_, FolderEvent::Open) => (FolderFetch::Fetching, Some(FolderEffect::Fetch)),
            (FolderFetch::Fetching, FolderEvent::Done) => (FolderFetch::Fetched { at: now }, None),
            (FolderFetch::Fetching, FolderEvent::Refused(why)) => (FolderFetch::Refused(why), None),
            (FolderFetch::Fetched { .. }, FolderEvent::Forget) => (FolderFetch::Unfetched, None),
            _ => (self.clone(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + TimeDelta::seconds(second as i64)
    }

    #[test]
    fn opening_fetches_unless_fetched_a_moment_ago() {
        let fetched = FolderFetch::Fetched { at: t(0) };
        let cases = [
            (FolderFetch::Unfetched, 0, true),
            (fetched.clone(), 59, false),
            (fetched.clone(), 60, true),
            (fetched.clone(), 600, true),
            // A clock that went backwards fetches rather than trusting the future.
            (FolderFetch::Fetched { at: t(100) }, 0, true),
            (FolderFetch::Refused("no".into()), 0, true),
            (FolderFetch::Fetching, 0, false),
        ];
        for (from, at, fetch) in cases {
            let (to, effect) = from.step(FolderEvent::Open, t(at));
            assert_eq!(effect.is_some(), fetch, "{from:?} at {at}");
            let want = if fetch {
                FolderFetch::Fetching
            } else {
                from.clone()
            };
            assert_eq!(to, want, "{from:?} at {at}");
        }
    }

    #[test]
    fn a_fetch_ends_fetched_or_refused() {
        let (to, fx) = FolderFetch::Fetching.step(FolderEvent::Done, t(7));
        assert_eq!((to, fx), (FolderFetch::Fetched { at: t(7) }, None));
        let (to, fx) = FolderFetch::Fetching.step(FolderEvent::Refused("no".into()), t(7));
        assert_eq!((to, fx), (FolderFetch::Refused("no".into()), None));
    }

    #[test]
    fn forgetting_makes_the_next_opening_fetch() {
        let (forgotten, _) = FolderFetch::Fetched { at: t(0) }.step(FolderEvent::Forget, t(1));
        let (_, fx) = forgotten.step(FolderEvent::Open, t(2));
        assert_eq!(fx, Some(FolderEffect::Fetch));
    }

    #[test]
    fn stray_events_do_nothing() {
        for from in [FolderFetch::Unfetched, FolderFetch::Fetched { at: t(0) }] {
            assert_eq!(from.step(FolderEvent::Done, t(1)), (from.clone(), None));
            assert_eq!(
                from.step(FolderEvent::Refused("x".into()), t(1)),
                (from, None)
            );
        }
    }
}
