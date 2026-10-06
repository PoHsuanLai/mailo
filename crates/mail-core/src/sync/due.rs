//! Which accounts a timed pass in the window syncs.
//!
//! The window wakes at the shortest interval any account asks for, [`super::poll_interval`]. A
//! wake syncs only the accounts whose own interval has passed: one Graph account polls every
//! minute, and the acct_imap() accounts beside it want a pass every five. A pass the user asks for is
//! not a wake and still syncs them all, through [`super::run`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mail_domain::{Incoming, WatchMode};
use mail_runtime::{OAuthRegistry, platform_secrets};
use mail_store::SqliteStore;
use porter_core::AccountId;

/// What an account that offers IDLE is polled at, since no connection is held open for it.
pub const IDLE: Duration = Duration::from_secs(300);

/// How often one account wants a pass.
pub fn every(watch: &WatchMode) -> Duration {
    match watch {
        WatchMode::Poll { every } => *every,
        WatchMode::Idle => IDLE,
    }
}

/// Each account with a server, and how often it wants a pass.
///
/// An account that keeps its mail here is left out: it has nothing to fetch, so it is never due.
pub fn intervals(store: &SqliteStore) -> Vec<(AccountId, Duration)> {
    super::configured(store)
        .unwrap_or_default()
        .into_iter()
        .filter(|account| !matches!(account.plan.incoming, Incoming::Local))
        .map(|account| (account.id, every(&account.caps.watch)))
        .collect()
}

/// The accounts whose own interval has passed since their last pass, and any never synced.
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
            last.get(account)
                .is_none_or(|at| now.saturating_duration_since(*at) >= *every)
        })
        .map(|(account, _)| account.clone())
        .collect()
}

/// One pass over the accounts in `due` and no others, as [`super::run`] does it for all of them.
pub fn run_due(
    store: Arc<SqliteStore>,
    now: chrono::DateTime<chrono::Utc>,
    due: &[AccountId],
    hooks: super::report::Hooks<'_>,
) -> Result<Vec<super::report::PassEnd>, String> {
    let registry = OAuthRegistry::load_default().map_err(|e| e.to_string())?;
    super::run_all(
        store,
        platform_secrets(),
        &registry,
        now,
        super::Mode::Once,
        super::Announce::Quietly,
        &super::Scope {
            due: &|account| due.contains(&account),
            kept: &crate::offline::load_default(),
        },
        hooks,
    )
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

    #[test]
    fn an_account_that_offers_idle_is_polled_every_five_minutes() {
        assert_eq!(every(&WatchMode::Idle), secs(300));
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
}
