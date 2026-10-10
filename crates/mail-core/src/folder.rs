//! Folders: listing them, and creating, renaming, deleting and following them.
//!
//! The same calls serve the command line and the window. [`change`] decides with
//! [`mail_domain::folder::plan`], writes the local half at once and queues the server's half in
//! the outbox, so a folder made on a train is there immediately and on the server after the
//! next sync — or put back, if the server refuses it for good.

use crate::error::{CoreError, Logged};
use chrono::{DateTime, Utc};
use mail_domain::folder::{FolderContents, FolderCtx, plan};
use mail_domain::{Applied, Folder, FolderError, FolderWork, Incoming, MailboxRef};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// Why a folder change did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The change itself cannot be made: the name, the folder, the protocol.
    #[error(transparent)]
    Folder(#[from] FolderError),
    /// No configured account has this id.
    #[error("no such account")]
    NoAccount,
    /// The store could not be read or written; nothing was changed.
    #[error("{0}")]
    Store(String),
}

/// Make a folder change: here at once, on the server when the outbox next drains.
///
/// Returns what was applied, so a caller can say what happened and, like any other op, undo it
/// with `inverse`. A refusal changes nothing.
pub fn change(
    store: &SqliteStore,
    account: AccountId,
    work: FolderWork,
    now: DateTime<Utc>,
) -> Result<Applied, Refusal> {
    let configured = crate::sync::configured(store)
        .map_err(|e| Refusal::Store(e.to_string()))?
        .into_iter()
        .find(|c| c.id == account)
        .ok_or(Refusal::NoAccount)?;
    let failed = |e: mail_store::StoreError| Refusal::Store(e.to_string());
    let folders = store.folders(account.clone()).map_err(failed)?;
    let labels = store.labels(account.clone()).map_err(failed)?;
    let contents = match &work {
        // Only a delete reads it, and only a delete needs the cost of asking.
        FolderWork::Delete { path, .. } => store
            .folder_contents(&MailboxRef {
                account: account.clone(),
                path: path.clone(),
            })
            .map_err(failed)?,
        _ => FolderContents::default(),
    };
    let applied = plan(
        &work,
        &FolderCtx {
            account: account.clone(),
            incoming: &configured.plan.incoming,
            caps: &configured.caps,
            folders: &folders,
            labels: &labels,
            contents: &contents,
        },
    )?;
    store
        .apply(account.clone(), &applied.forward)
        .map_err(failed)?;
    if let Some(intent) = applied.remote.clone() {
        // Unlike a flag change, a folder that exists only here is a folder the server will
        // contradict at the next listing. If it cannot be queued, it is not made at all.
        if let Err(e) = store.enqueue(account.clone(), intent, &applied.inverse, now) {
            store
                .apply(account, &applied.inverse)
                .or_log("a folder change could not be undone after it failed to queue");
            return Err(failed(e));
        }
    }
    Ok(applied)
}

/// What one account's folder listing has to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    /// A POP3 account has the one mailbox, and no folders.
    Pop3,
    /// A local account is kept on this computer: labels only, no server folders.
    Local,
    /// A server account whose folders have not been listed yet.
    Unlisted,
    /// The folders the server listed.
    Folders(Vec<Folder>),
}

/// One account and its [`Listing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFolders {
    pub address: String,
    pub listing: Listing,
}

/// Every folder, per account; `account` narrows it to one.
pub fn list(
    store: &SqliteStore,
    account: Option<AccountId>,
) -> Result<Vec<AccountFolders>, CoreError> {
    let accounts = crate::sync::configured(store)?;
    if accounts.is_empty() {
        return Err(CoreError::NoAccounts);
    }
    let mut out = Vec::new();
    for configured in accounts
        .iter()
        .filter(|c| account.clone().is_none_or(|a| a == c.id))
    {
        let listing = match configured.plan.incoming {
            Incoming::Pop3 { .. } => Listing::Pop3,
            Incoming::Local => Listing::Local,
            Incoming::Imap { .. } | Incoming::Graph | Incoming::Jmap { .. } => {
                let folders = store.folders(configured.id.clone())?;
                if folders.is_empty() {
                    Listing::Unlisted
                } else {
                    Listing::Folders(folders)
                }
            }
        };
        out.push(AccountFolders {
            address: configured.address.clone(),
            listing,
        });
    }
    Ok(out)
}
