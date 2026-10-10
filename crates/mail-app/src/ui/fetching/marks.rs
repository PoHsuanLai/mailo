//! What a row, a button and a face show of the links behind them: pure mappings from fetch
//! state to the small marks quire draws, so each can be tested with a table.

use super::status::{clock, not_updated_since};
use chrono::{DateTime, TimeZone, Utc};
use ds::prelude::Availability;
use mail_core::fetch::{FolderFetch, Link, Pause};
use mail_core::sync::fresh::{self, Freshness, Pace};

/// What a row carries at its end for the work or the trouble behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// Nothing to say.
    Quiet,
    /// Work is going on: the small spinner.
    Busy,
    /// Something is wrong: a warning glyph, and why, as its hover text and its label.
    Warn(String),
    /// The server cannot be reached: Mail's offline glyph, and why, as the warning has it.
    Offline(String),
}

/// The sync button's availability: busy while any account in view is fetching.
pub fn sync_availability(links: &[&Link]) -> Availability {
    if links.iter().any(|link| link.is_busy()) {
        Availability::Busy
    } else {
        Availability::Enabled
    }
}

/// An account's mark: the spinner while its pass runs, else a warning with the reason when it
/// needs the person, could not update everything, or has not been updated for longer than its
/// `pace` allows.
pub fn account_mark<Tz: TimeZone>(link: &Link, pace: Pace, now: DateTime<Utc>, zone: &Tz) -> Mark
where
    Tz::Offset: std::fmt::Display,
{
    if let Freshness::Stale { since } = fresh::of_link(link, pace, now) {
        return Mark::Warn(format!("{}.", not_updated_since(since, now, zone)));
    }
    match link {
        Link::Syncing { .. } => Mark::Busy,
        Link::Fresh => Mark::Quiet,
        Link::NeedsSignIn { .. } => Mark::Warn("Sign in again to keep receiving mail.".to_owned()),
        Link::NeedsAllow { .. } => {
            Mark::Warn("Allow Mail again to keep receiving mail.".to_owned())
        }
        Link::Broken { why, .. } => Mark::Warn(why.clone()),
        Link::Waiting { until, why, .. } => {
            let at = clock(*until, zone);
            match why {
                Pause::Unreachable => Mark::Offline(format!(
                    "Can\u{2019}t reach the server. Trying again at {at}."
                )),
                Pause::Throttled => Mark::Warn(format!(
                    "The server asked to slow down. Trying again at {at}."
                )),
                Pause::ServerBusy => {
                    Mark::Warn(format!("The server is busy. Trying again at {at}."))
                }
            }
        }
        Link::Current { trouble, .. } => match trouble.as_slice() {
            [] => Mark::Quiet,
            [one] => Mark::Warn(match &one.mailbox {
                Some(folder) => format!("{folder} couldn\u{2019}t update: {}", one.why),
                None => one.why.clone(),
            }),
            many => Mark::Warn(format!("{} folders couldn\u{2019}t update.", many.len())),
        },
    }
}

/// A folder's mark: the spinner while it is fetched, a quiet warning with the server's words
/// when it said no.
pub fn folder_mark(fetch: &FolderFetch) -> Mark {
    match fetch {
        FolderFetch::Fetching => Mark::Busy,
        FolderFetch::Refused(why) => Mark::Warn(why.clone()),
        FolderFetch::Unfetched | FolderFetch::Fetched { .. } => Mark::Quiet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mail_core::fetch::{First, Live, Step, Trouble};
    use mail_domain::Retry;

    fn at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 30, 0).unwrap()
    }

    fn five() -> Pace {
        Pace::of_watch(&mail_domain::WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        })
    }

    fn syncing() -> Link {
        Link::Syncing {
            first: First::No,
            step: Step::Connecting,
            count: None,
            after: Box::new(Link::Fresh),
        }
    }

    fn current(folders: &[Option<&str>]) -> Link {
        Link::Current {
            at: at(),
            live: Live::Polling,
            trouble: folders
                .iter()
                .map(|mailbox| Trouble {
                    mailbox: mailbox.map(str::to_owned),
                    retry: Retry::Now,
                    why: "no room".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_sync_button_is_busy_while_any_link_runs() {
        let (idle, busy) = (current(&[]), syncing());
        assert_eq!(sync_availability(&[]), Availability::Enabled);
        assert_eq!(sync_availability(&[&idle]), Availability::Enabled);
        assert_eq!(sync_availability(&[&idle, &busy]), Availability::Busy);
    }

    #[test]
    fn an_account_shows_work_before_trouble_and_trouble_with_its_reason() {
        let said = |link: &Link| account_mark(link, five(), at(), &Utc);
        assert_eq!(said(&syncing()), Mark::Busy);
        assert_eq!(said(&Link::Fresh), Mark::Quiet);
        assert_eq!(said(&current(&[])), Mark::Quiet);
        let first = Link::Broken {
            why: "The mailbox is gone.".into(),
            first: First::No,
        };
        assert_eq!(said(&first), Mark::Warn("The mailbox is gone.".into()));
        let signed_out = Link::NeedsSignIn {
            why: "x".into(),
            first: First::No,
        };
        assert!(matches!(said(&signed_out), Mark::Warn(text) if text.starts_with("Sign in")));
        let waiting = Link::Waiting {
            until: at(),
            why: Pause::Unreachable,
            failures: 1,
            first: First::No,
        };
        assert_eq!(
            said(&waiting),
            Mark::Offline("Can\u{2019}t reach the server. Trying again at 12:30.".into())
        );
    }

    #[test]
    fn current_with_trouble_names_the_folder_or_counts_them() {
        let said = |link: &Link| account_mark(link, five(), at(), &Utc);
        assert_eq!(
            said(&current(&[Some("Work")])),
            Mark::Warn("Work couldn\u{2019}t update: no room".into())
        );
        assert_eq!(
            said(&current(&[Some("A"), None])),
            Mark::Warn("2 folders couldn\u{2019}t update.".into())
        );
    }

    /// The same rule as the status line's: an account resting at "current" for longer than its
    /// schedule allows is marked, with since when.
    #[test]
    fn an_account_past_its_schedule_is_marked_with_since_when() {
        let pushed = Link::Current {
            at: at(),
            live: Live::Pushed,
            trouble: vec![],
        };
        let after = |minutes| at() + chrono::TimeDelta::minutes(minutes);
        let idle = Pace::of_watch(&mail_domain::WatchMode::Idle);
        assert_eq!(account_mark(&pushed, idle, after(30), &Utc), Mark::Quiet);
        assert_eq!(
            account_mark(&pushed, idle, after(36), &Utc),
            Mark::Warn("Not updated since 12:30.".into())
        );
        assert_eq!(
            account_mark(&current(&[]), five(), after(16), &Utc),
            Mark::Warn("Not updated since 12:30.".into())
        );
    }

    #[test]
    fn a_folder_shows_its_fetch() {
        assert_eq!(folder_mark(&FolderFetch::Fetching), Mark::Busy);
        assert_eq!(folder_mark(&FolderFetch::Unfetched), Mark::Quiet);
        assert_eq!(folder_mark(&FolderFetch::Fetched { at: at() }), Mark::Quiet);
        assert_eq!(
            folder_mark(&FolderFetch::Refused("No such folder.".into())),
            Mark::Warn("No such folder.".into())
        );
    }
}
