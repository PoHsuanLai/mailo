//! Where each account's link begins, and how the set of links follows the set of accounts.

use chrono::{DateTime, Utc};
use mail_core::fetch::{Link, Live};
use mail_core::{SqliteStore, Store};
use mail_domain::{Filter, JMAP_ALL, MailboxRef};
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// When the last pass of `account` finished, as the store recorded it.
pub(super) fn last_synced(store: &SqliteStore, account: AccountId) -> Option<DateTime<Utc>> {
    store.last_synced(account).ok().flatten()
}

/// Whether the store holds anything of `account`'s: a cursor, or mail that an import left
/// without one.
fn has_mail(store: &SqliteStore, account: AccountId, now: DateTime<Utc>) -> bool {
    let cursor = |path: &str| {
        store
            .cursor(&MailboxRef {
                account: account.clone(),
                path: path.to_owned(),
            })
            .ok()
            .flatten()
            .is_some()
    };
    cursor("INBOX")
        || cursor(JMAP_ALL)
        || store
            .count(&Filter::Account(account), now)
            .is_ok_and(|threads| threads > 0)
}

/// The link an account begins at: knowing nothing, or resting after an earlier window's pass.
///
/// `last` is when the store says that pass finished, if it does. Without a time the account is
/// said to be current as of `now`, so that it is not shown as stale for a reason nobody can
/// give. Either way it polls again at once: a window opening on mail that is minutes old is
/// what the old loop answered with a pass a beat after the first paint.
pub(super) fn link_for(last: Option<DateTime<Utc>>, mail: bool, now: DateTime<Utc>) -> Link {
    match (last, mail) {
        (None, false) => Link::Fresh,
        (last, _) => Link::Current {
            at: last.unwrap_or(now).min(now),
            trouble: Vec::new(),
            live: Live::Polling,
        },
    }
}

/// [`link_for`], asking the store.
pub(super) fn probe(store: &SqliteStore, account: AccountId, now: DateTime<Utc>) -> Link {
    link_for(
        last_synced(store, account.clone()),
        has_mail(store, account, now),
        now,
    )
}

/// Every account that has a server, with the link it begins at.
pub(super) fn initial(
    store: &SqliteStore,
    accounts: &[(AccountId, Duration)],
    now: DateTime<Utc>,
) -> BTreeMap<AccountId, Link> {
    accounts
        .iter()
        .map(|(id, _)| (id.clone(), probe(store, id.clone(), now)))
        .collect()
}

/// What changed between the accounts that have links and the accounts that exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Changes {
    /// Accounts to give a link.
    pub added: Vec<AccountId>,
    /// Accounts whose link goes, with any pass it is running.
    pub removed: Vec<AccountId>,
}

/// The links to add and drop so that `known` matches `wanted`.
pub(super) fn reconcile(known: &BTreeSet<AccountId>, wanted: &[(AccountId, Duration)]) -> Changes {
    let wanted_ids: BTreeSet<AccountId> = wanted.iter().map(|(id, _)| id.clone()).collect();
    Changes {
        added: wanted_ids.difference(known).cloned().collect(),
        removed: known.difference(&wanted_ids).cloned().collect(),
    }
}

#[cfg(test)]
mod tests;
