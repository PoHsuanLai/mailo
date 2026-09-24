//! Which accounts a timed pass in the window syncs.
//!
//! The window wakes at the shortest interval any account asks for, [`super::poll_interval`].
//! Every wake used to sync every account, so one Graph account, which polls every minute, had
//! every IMAP account beside it polled every minute too, five times what its server asked for.
//! A wake now syncs only the accounts whose own interval has passed. A pass the user asks for is
//! not a wake and still syncs them all, through [`super::run`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mail_domain::{AccountId, Incoming, WatchMode};
use mail_runtime::{KeyringSecrets, OAuthRegistry};
use mail_store::SqliteStore;

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
        .map(|(account, _)| *account)
        .collect()
}

/// One pass over the accounts in `due` and no others, as [`super::run`] does it for all of them.
pub fn run_due(
    store: Arc<SqliteStore>,
    now: chrono::DateTime<chrono::Utc>,
    due: &[AccountId],
) -> Result<super::Ran, String> {
    let registry = OAuthRegistry::load_default().map_err(|e| e.to_string())?;
    super::run_all(
        store,
        Arc::new(KeyringSecrets),
        &registry,
        now,
        super::Mode::Once,
        super::Announce::Quietly,
        &|account| due.contains(&account),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: AccountId =
        AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e1"));
    const IMAP: AccountId =
        AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e2"));
    const PUSHED: AccountId =
        AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e3"));
    const NEW: AccountId =
        AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e4"));

    #[test]
    fn each_account_is_due_at_its_own_interval() {
        let intervals = [
            (GRAPH, every(&WatchMode::Poll { every: secs(60) })),
            (IMAP, every(&WatchMode::Poll { every: secs(300) })),
            (PUSHED, every(&WatchMode::Idle)),
            (NEW, secs(300)),
        ];
        let start = Instant::now();
        // Every account but `NEW` last synced at `start`; `NEW` never has.
        let last: HashMap<AccountId, Instant> = [GRAPH, IMAP, PUSHED]
            .into_iter()
            .map(|account| (account, start))
            .collect();
        let table: [(u64, &[AccountId]); 6] = [
            (0, &[NEW]),
            (59, &[NEW]),
            (60, &[GRAPH, NEW]),
            (120, &[GRAPH, NEW]),
            (299, &[GRAPH, NEW]),
            (300, &[GRAPH, IMAP, PUSHED, NEW]),
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
        let intervals = [(GRAPH, secs(60)), (IMAP, secs(300))];
        assert_eq!(
            due(&intervals, &HashMap::new(), Instant::now()),
            [GRAPH, IMAP]
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
