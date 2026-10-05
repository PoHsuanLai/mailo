//! Removing an account from this computer: its saved sign-in, then everything the database keeps
//! of it.
//!
//! The keyring goes first. A sign-in that cannot be forgotten stops the removal with nothing
//! deleted, so the account is still there to try again; the other order would leave a password
//! in the keyring that no account names any more, which nothing would ever find to clean up.
//! The database goes last ([`SqliteStore::remove_account`]): every row that names the account,
//! and the stored messages and attachment parts nothing else uses. Nothing on the server is
//! touched, and keys and certificates stay: they are the user's, not the account's.

use mail_domain::{AccountId, AccountPlan, Incoming, SecretKey, SecretPurpose};
use mail_runtime::Secrets;
use mail_store::{Freed, SqliteStore};
use rusqlite::OptionalExtension as _;

/// Every secret kept under an account's id. Keys and certificates are kept under their
/// fingerprint, not the account, and are not among them.
const PURPOSES: [SecretPurpose; 4] = [
    SecretPurpose::IncomingPassword,
    SecretPurpose::OutgoingPassword,
    SecretPurpose::OAuthRefresh,
    SecretPurpose::AddressBook,
];

/// An account that was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The address it had, for the front-end to say what went.
    pub address: String,
    /// The stored messages and parts that went with it.
    pub freed: Freed,
}

/// Why an account was not removed. Nothing was deleted in any of these cases.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// No account has this id: removed already, or never added.
    #[error("no such account")]
    Unknown,
    /// The mail kept on this computer and nowhere else. It holds imported mail, which removing
    /// it would delete for good.
    #[error("the account that keeps mail on this computer cannot be removed")]
    Local,
    /// The keyring would not forget a sign-in, so the account stays as it was.
    #[error("the keyring would not forget the sign-in: {0}")]
    Keyring(String),
    /// The database could not be read or written.
    #[error("the database refused: {0}")]
    Store(String),
}

/// Remove `account` and everything kept of it here. See the module's words for the order.
pub fn remove(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    account: AccountId,
) -> Result<Removed, RemoveError> {
    let id = account.to_string();
    let (address, plan) = {
        let db = store.connection();
        db.query_row(
            "SELECT address, plan FROM accounts WHERE id = ?1",
            [&id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| RemoveError::Store(e.to_string()))?
        .ok_or(RemoveError::Unknown)?
    };
    // A plan that no longer reads is still an account with a server: only a readable `Local`
    // is refused.
    if serde_json::from_str::<AccountPlan>(&plan)
        .is_ok_and(|plan| matches!(plan.incoming, Incoming::Local))
    {
        return Err(RemoveError::Local);
    }
    for purpose in PURPOSES {
        secrets
            .forget(&SecretKey { account, purpose })
            .map_err(|e| RemoveError::Keyring(e.to_string()))?;
    }
    match store
        .remove_account(account)
        .map_err(|e| RemoveError::Store(e.to_string()))?
    {
        None => Err(RemoveError::Unknown),
        Some(freed) => Ok(Removed { address, freed }),
    }
}

#[cfg(test)]
#[path = "remove_tests.rs"]
mod tests;
