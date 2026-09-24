//! Folders: listing them, and creating, renaming, deleting and following them.
//!
//! The same two calls serve `mailo folder` and the window. [`change`] decides with
//! [`mail_domain::folder::plan`], writes the local half at once and queues the server's half in
//! the outbox, so a folder made on a train is there immediately and on the server after the
//! next sync — or put back, if the server refuses it for good.

use chrono::{DateTime, Utc};
use mail_domain::folder::{FolderContents, FolderCtx, plan};
use mail_domain::{
    AccountId, Applied, Folder, FolderError, FolderWork, Holds, Incoming, MailboxRef, NonEmpty,
    Subscription,
};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

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
        .map_err(Refusal::Store)?
        .into_iter()
        .find(|c| c.id == account)
        .ok_or(Refusal::NoAccount)?;
    let failed = |e: mail_store::StoreError| Refusal::Store(e.to_string());
    let folders = store.folders(account).map_err(failed)?;
    let labels = store.labels(account).map_err(failed)?;
    let contents = match &work {
        // Only a delete reads it, and only a delete needs the cost of asking.
        FolderWork::Delete { path, .. } => store
            .folder_contents(&MailboxRef {
                account,
                path: path.clone(),
            })
            .map_err(failed)?,
        _ => FolderContents::default(),
    };
    let applied = plan(
        &work,
        &FolderCtx {
            account,
            incoming: &configured.plan.incoming,
            caps: &configured.caps,
            folders: &folders,
            labels: &labels,
            contents: &contents,
        },
    )?;
    store.apply(account, &applied.forward).map_err(failed)?;
    if let Some(intent) = applied.remote.clone() {
        // Unlike a flag change, a folder that exists only here is a folder the server will
        // contradict at the next listing. If it cannot be queued, it is not made at all.
        if let Err(e) = store.enqueue(account, intent, &applied.inverse, now) {
            let _ = store.apply(account, &applied.inverse);
            return Err(failed(e));
        }
    }
    Ok(applied)
}

/// `mailo folder list [account]`: every folder, per account.
pub fn list(store: &SqliteStore, account: Option<AccountId>) -> Result<String, String> {
    let accounts = crate::sync::configured(store)?;
    if accounts.is_empty() {
        return Err("no accounts. Add one with: mailo account add <address>".to_owned());
    }
    let mut out = String::new();
    for configured in accounts
        .iter()
        .filter(|c| account.is_none_or(|a| a == c.id))
    {
        let _ = writeln!(out, "{}", configured.address);
        match configured.plan.incoming {
            Incoming::Pop3 { .. } => {
                let _ = writeln!(out, "  POP3 has one mailbox, and no folders\n");
                continue;
            }
            Incoming::Local => {
                let _ = writeln!(
                    out,
                    "  kept on this computer: no server folders, only labels\n"
                );
                continue;
            }
            Incoming::Imap { .. } | Incoming::Graph => {}
        }
        let folders = store.folders(configured.id).map_err(|e| e.to_string())?;
        if folders.is_empty() {
            let _ = writeln!(
                out,
                "  no folders listed yet; `mailo sync` asks the server\n"
            );
            continue;
        }
        for folder in &folders {
            let _ = writeln!(out, "  {}", line(folder));
        }
        out.push('\n');
    }
    Ok(out)
}

/// One folder, as `list` prints it.
fn line(folder: &Folder) -> String {
    let mut notes = Vec::new();
    if let Some(special) = folder.protected() {
        notes.push(special.name().to_lowercase());
    }
    if folder.holds == Holds::FoldersOnly {
        notes.push("holds folders only".to_owned());
    }
    if folder.subscription == Subscription::Unsubscribed {
        notes.push("not subscribed".to_owned());
    }
    if notes.is_empty() {
        folder.path.clone()
    } else {
        format!("{}  ({})", folder.path, notes.join(", "))
    }
}

/// What `mailo folder new|rename|delete|subscribe|unsubscribe` prints once it has happened here.
pub fn said(work: &FolderWork, address: &str) -> String {
    let what = match work {
        FolderWork::Create { path } => format!("created {path}"),
        FolderWork::Rename { from, to } => format!("renamed {from} to {to}"),
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Allow,
        } => format!("deleted {path} and anything in it"),
        FolderWork::Delete { path, .. } => format!("deleted {path}"),
        FolderWork::Subscribe {
            path,
            subscription: Subscription::Subscribed,
        } => format!("subscribed to {path}"),
        FolderWork::Subscribe { path, .. } => format!("unsubscribed from {path}"),
    };
    format!("{what} on {address}; the server is told on the next sync\n")
}
