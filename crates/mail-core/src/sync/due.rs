//! Which accounts a timed pass in the window syncs.
//!
//! The window wakes at the shortest interval any account asks for, [`super::poll_interval`]. A
//! wake syncs only the accounts whose own interval has passed: one Graph account polls every
//! minute, and the acct_imap() accounts beside it want a pass every five. A pass the user asks for is
//! not a wake and still syncs them all, through [`SyncOps::run`](crate::SyncOps).
//!
//! Each account's interval is the one its server asks for unless the person chose another on
//! the account's page ([`crate::offline::Every`]). One checked manually is still an account with
//! a server, so a person's Sync reaches it; its interval is [`MANUALLY`], which no timer reaches
//! and the scheduler starts no pass at, the first one included.

use crate::error::{CoreError, Logged};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::offline::{Every, Kept};
use mail_domain::{Incoming, WatchMode};
use mail_store::SqliteStore;
use porter_core::AccountId;

pub use mail_runtime::schedule::MANUALLY;

/// What an account that offers IDLE is polled at, since no connection is held open for it.
pub const IDLE: Duration = Duration::from_secs(300);

/// How often one account wants a pass.
pub fn every(watch: &WatchMode) -> Duration {
    match watch {
        WatchMode::Poll { every } => *every,
        WatchMode::Idle => IDLE,
    }
}

/// How often one account is checked, given what the person chose: the server's own interval on
/// [`Every::Automatic`], the minutes chosen, or [`MANUALLY`].
pub fn chosen(watch: &WatchMode, chose: Every) -> Duration {
    match chose {
        Every::Automatic => every(watch),
        // Never under a minute, whatever a hand-edited file says: a zero would be a pass at every
        // turn of the loop.
        Every::Minutes(minutes) => Duration::from_secs(60 * u64::from(minutes.max(1))),
        Every::Manually => MANUALLY,
    }
}

/// Each account with a server, and how often it wants a pass, as the setting in the user's config
/// directory chooses ([`intervals_with`]).
pub fn intervals(store: &SqliteStore) -> Vec<(AccountId, Duration)> {
    intervals_with(store, &crate::offline::load_default())
}

/// Each account with a server, and how often it wants a pass, as `kept` chooses.
///
/// An account that keeps its mail here is left out: it has nothing to fetch, so it is never due.
pub fn intervals_with(store: &SqliteStore, kept: &Kept) -> Vec<(AccountId, Duration)> {
    super::configured(store)
        .or_log_default("the accounts could not be read")
        .into_iter()
        .filter(|account| !matches!(account.plan.incoming, Incoming::Local))
        .map(|account| {
            let wants = chosen(&account.caps.watch, kept.every_of(&account.id));
            (account.id, wants)
        })
        .collect()
}

/// The accounts whose own interval has passed since their last pass, and any never synced, but
/// for one checked [`MANUALLY`], which no wake is for.
///
/// `last` is when each account's last pass started, which is the moment its interval counts
/// from. Kept by the window in memory rather than in the store: a window just opened syncs every
/// account once anyway, and a pass that failed still counts, or a server that is down would be
/// asked again at every wake rather than at its own interval.
pub fn due(
    intervals: &[(AccountId, Duration)],
    last: &HashMap<AccountId, Instant>,
    now: Instant,
) -> Vec<AccountId> {
    intervals
        .iter()
        .filter(|(account, every)| {
            *every != MANUALLY
                && last
                    .get(account)
                    .is_none_or(|at| now.saturating_duration_since(*at) >= *every)
        })
        .map(|(account, _)| account.clone())
        .collect()
}

impl crate::mail::SyncOps<'_> {
    /// One pass over the accounts in `due` and no others, as [`SyncOps::run`](crate::SyncOps) does
    /// it for all of them.
    pub async fn run_due(
        &self,
        due: &[AccountId],
        hooks: super::report::Hooks<'_>,
    ) -> Result<Vec<super::report::PassEnd>, CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        super::run_all(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            mail.now(),
            super::Mode::Once,
            &super::Scope {
                due: &|account| due.contains(&account),
                kept: &crate::offline::load_default(),
            },
            hooks,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;

    fn acct_graph() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e1"))
    }
    fn acct_imap() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e2"))
    }
    fn acct_pushed() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e3"))
    }
    fn acct_new() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e4"))
    }

    #[test]
    fn each_account_is_due_at_its_own_interval() {
        let intervals = [
            (acct_graph(), every(&WatchMode::Poll { every: secs(60) })),
            (acct_imap(), every(&WatchMode::Poll { every: secs(300) })),
            (acct_pushed(), every(&WatchMode::Idle)),
            (acct_new(), secs(300)),
        ];
        let start = Instant::now();
        // Every account but `acct_new()` last synced at `start`; `acct_new()` never has.
        let last: HashMap<AccountId, Instant> = [acct_graph(), acct_imap(), acct_pushed()]
            .into_iter()
            .map(|account| (account, start))
            .collect();
        let table: [(u64, &[AccountId]); 6] = [
            (0, &[acct_new()]),
            (59, &[acct_new()]),
            (60, &[acct_graph(), acct_new()]),
            (120, &[acct_graph(), acct_new()]),
            (299, &[acct_graph(), acct_new()]),
            (300, &[acct_graph(), acct_imap(), acct_pushed(), acct_new()]),
        ];
        for (elapsed, expected) in table {
            assert_eq!(
                due(&intervals, &last, start + secs(elapsed)),
                expected,
                "{elapsed} s after the last pass"
            );
        }
    }

    #[test]
    fn a_window_just_opened_syncs_every_account() {
        let intervals = [(acct_graph(), secs(60)), (acct_imap(), secs(300))];
        assert_eq!(
            due(&intervals, &HashMap::new(), Instant::now()),
            [acct_graph(), acct_imap()]
        );
    }

    /// What the account page offers, as the interval the scheduler is handed.
    #[test]
    fn each_choice_is_the_interval_it_says() {
        let poll = WatchMode::Poll { every: secs(60) };
        let table: [(&WatchMode, Every, Duration); 9] = [
            (&poll, Every::Automatic, secs(60)),
            (&WatchMode::Idle, Every::Automatic, IDLE),
            (&WatchMode::Idle, Every::Minutes(1), secs(60)),
            (&poll, Every::Minutes(5), secs(300)),
            (&poll, Every::Minutes(15), secs(900)),
            (&poll, Every::Minutes(30), secs(1800)),
            (&WatchMode::Idle, Every::Minutes(60), secs(3600)),
            (&poll, Every::Minutes(0), secs(60)),
            (&WatchMode::Idle, Every::Manually, MANUALLY),
        ];
        for (watch, chose, want) in table {
            assert_eq!(chosen(watch, chose), want, "{watch:?}, {chose:?}");
        }
    }

    /// One checked manually is never due, not even before its first pass; the others beside it
    /// are due as they always were.
    #[test]
    fn an_account_checked_manually_is_never_due() {
        let intervals = [
            (acct_graph(), chosen(&WatchMode::Idle, Every::Manually)),
            (acct_imap(), chosen(&WatchMode::Idle, Every::Minutes(15))),
            (acct_new(), chosen(&WatchMode::Idle, Every::Manually)),
        ];
        let start = Instant::now();
        let last: HashMap<AccountId, Instant> = [acct_graph(), acct_imap()]
            .into_iter()
            .map(|account| (account, start))
            .collect();
        let table: [(u64, &[AccountId]); 4] = [
            (0, &[]),
            (899, &[]),
            (900, &[acct_imap()]),
            (365 * 24 * 3600, &[acct_imap()]),
        ];
        for (elapsed, expected) in table {
            assert_eq!(
                due(&intervals, &last, start + secs(elapsed)),
                expected,
                "{elapsed} s after the last pass"
            );
        }
    }

    #[test]
    fn an_account_that_offers_idle_is_polled_every_five_minutes() {
        assert_eq!(every(&WatchMode::Idle), secs(300));
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
}
