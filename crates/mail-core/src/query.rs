//! The store's half of the search language: the names a typed `label:` and `account:` stand for.
//!
//! Reading a typed line into a [`Filter`](mail_domain::Filter) needs no store and is
//! `mail_logic::query`; the names it resolves against are asked of the store here, and handed to
//! its `Known`.

use crate::error::Logged;
use mail_domain::LabelId;
use porter_core::AccountId;

/// Every label name in the store, with the label that bears it.
///
/// The other half of `label:`, kept next to the parser it feeds rather than at each surface,
/// because that is where the two drifted: `mailo search` passed a resolver and the window passed
/// `parse`, whose resolver knows no names. `label:travel` therefore worked in the terminal and,
/// in the window, became a full-text search for the literal string — no results and no error.
///
/// A name can appear more than once: `UNIQUE (account, name)` is per account, so the same word
/// on two accounts is two labels, and someone typing it means both.
pub fn known_labels(store: &mail_store::SqliteStore) -> Vec<(String, LabelId)> {
    let accounts: Vec<porter_core::AccountId> = store
        .list_accounts()
        .or_log_default("the accounts could not be read for their labels")
        .into_iter()
        .map(|account| account.id)
        .collect();
    accounts
        .into_iter()
        .filter_map(|account| {
            store
                .labels(account)
                .or_log("an account's labels could not be read")
        })
        .flatten()
        .map(|l| (l.name, l.id))
        .collect()
}

/// Every account in the store, by its address: the other half of `account:`.
pub fn known_accounts(store: &mail_store::SqliteStore) -> Vec<(String, AccountId)> {
    store
        .list_accounts()
        .or_log_default("the accounts could not be read for their addresses")
        .into_iter()
        .map(|account| (account.address, account.id))
        .collect()
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
