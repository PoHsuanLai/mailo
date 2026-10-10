//! What the store says about accounts and the identities they send as.
//!
//! The rows themselves are the store's: callers ask for these values and give these values, and
//! never see the columns they are kept in.

use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{AccountCaps, AccountPlan, Identity};
use porter_core::AccountId;

/// One account as it is stored.
///
/// The plan and the capabilities are kept as JSON and decoded when read, and a row that no
/// longer decodes is still an account: the address and the id are good. So each is a `Result`
/// that the caller reads the way it needs to, where one that cannot read a plan would still list
/// the account and another would refuse to go on.
#[derive(Debug)]
pub struct StoredAccount {
    pub id: AccountId,
    pub address: String,
    pub plan: Result<AccountPlan, StoreError>,
    /// `None` when nothing has connected yet, and the capabilities have never been written.
    pub caps: Option<Result<AccountCaps, StoreError>>,
}

/// An account to write: all of it, or none of it.
#[derive(Debug, Clone, Copy)]
pub struct NewAccount<'a> {
    pub id: &'a AccountId,
    /// The address the account is found by, which is unique. Written as given.
    pub address: &'a str,
    pub plan: &'a AccountPlan,
    /// What the server is expected to support, which a real connection later replaces.
    pub caps: &'a AccountCaps,
    /// The identities to send as. One already stored is left as it is, so a signature the user
    /// set survives an account being added again.
    pub identities: &'a [Identity],
    /// When the account was created, kept as it was if the address is already here.
    pub at: DateTime<Utc>,
}
