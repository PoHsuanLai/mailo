//! Removing an account from this computer: its saved sign-in, then everything the database keeps
//! of it.
//!
//! The secrets go first, every one of the account's in one `Secrets::delete_account`. A sign-in
//! that cannot be forgotten stops the removal before the database is touched, so the account and
//! its mail are still there to try again; the other order would leave a password in the keyring
//! that no account names any more, which nothing would ever find to clean up. A store that
//! refuses part-way may have forgotten some already, and those stay forgotten: the account then
//! asks to sign in again, which is what [`RemoveError::Keyring`] says.
//! The database goes last ([`SqliteStore::remove_account`]): every row that names the account,
//! and the stored messages and attachment parts nothing else uses. Nothing on the server is
//! touched, and keys and certificates stay: they are the user's, not the account's.
//!
//! An account that is the desktop's accountd's (`AuthPlan::Granted`) has no secret here to forget:
//! removing it is withdrawing Mail's grant on it, which stops Mail using the account and leaves it
//! accountd's, and then the database. The grant goes first, for the same reason the secrets do: a
//! grant that could not be withdrawn stops the removal with the account still here, where the
//! other order would leave a grant nothing names and the next read of accountd's accounts would
//! bring the account straight back.

use mail_domain::Incoming;
use mail_runtime::AccountSecrets;
use mail_store::{Freed, SqliteStore};
use porter_core::AccountId;

/// An account that was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The address it had, for the front-end to say what went.
    pub address: String,
    /// The stored messages and parts that went with it.
    pub freed: Freed,
}

/// Why an account was not removed. In every case the account and its mail are still there; after
/// [`RemoveError::Keyring`] or [`RemoveError::Store`], some of its saved sign-ins may already be
/// forgotten (see the module's words), and it asks to sign in again.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// No account has this id: removed already, or never added.
    #[error("no such account")]
    Unknown,
    /// The mail kept on this computer and nowhere else. It holds imported mail, which removing
    /// it would delete for good.
    #[error("the account that keeps mail on this computer cannot be removed")]
    Local,
    /// The keyring would not forget a sign-in: the account and its mail stay, and a sign-in
    /// forgotten before the refusal stays forgotten.
    #[error(
        "the keyring would not forget a saved sign-in, so the account was not removed \
         (it may ask to sign in again): {0}"
    )]
    Keyring(String),
    /// The database could not be read or written, after the sign-ins were forgotten: the account
    /// and its mail stay, and it asks to sign in again.
    #[error(
        "the database refused, so the account was not removed (it will ask to sign in again): {0}"
    )]
    Store(String),
}

/// Remove `account` and everything kept of it here. See the module's words for the order.
pub async fn remove(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: AccountId,
) -> Result<Removed, RemoveError> {
    let stored = store
        .account(account.clone())
        .map_err(|e| RemoveError::Store(e.to_string()))?
        .ok_or(RemoveError::Unknown)?;
    let address = stored.address;
    let plan = stored.plan.ok();
    // A plan that no longer reads is still an account with a server: only a readable `Local`
    // is refused.
    if plan
        .as_ref()
        .is_some_and(|plan| matches!(plan.incoming, Incoming::Local))
    {
        return Err(RemoveError::Local);
    }
    match plan.as_ref().and_then(|plan| plan.grant()) {
        Some(grant) => {
            let link = secrets.link().ok_or_else(|| {
                RemoveError::Keyring(
                    "the desktop's account service is not reachable, so Mail's grant on the \
                     account cannot be withdrawn"
                        .to_owned(),
                )
            })?;
            link.revoke(grant)
                .await
                .map_err(|e| RemoveError::Keyring(e.to_string()))?;
        }
        None => secrets
            .forget_account(&account)
            .await
            .map_err(|e| RemoveError::Keyring(e.to_string()))?,
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
