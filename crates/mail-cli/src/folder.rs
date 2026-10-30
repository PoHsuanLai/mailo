//! `folder list` and the line the other `folder` commands answer with.

use mail_core::folder::{AccountFolders, Listing};
use mail_domain::{Folder, FolderWork, Holds, NonEmpty, Subscription};
use std::fmt::Write as _;

/// `mailo folder list [account]`: every folder, per account.
pub(super) fn listing(accounts: &[AccountFolders]) -> String {
    let mut out = String::new();
    for account in accounts {
        let _ = writeln!(out, "{}", account.address);
        match &account.listing {
            Listing::Pop3 => {
                let _ = writeln!(out, "  POP3 has one mailbox, and no folders\n");
            }
            Listing::Local => {
                let _ = writeln!(
                    out,
                    "  kept on this computer: no server folders, only labels\n"
                );
            }
            Listing::Unlisted => {
                let _ = writeln!(
                    out,
                    "  no folders listed yet; `mailo sync` asks the server\n"
                );
            }
            Listing::Folders(folders) => {
                for folder in folders {
                    let _ = writeln!(out, "  {}", line(folder));
                }
                out.push('\n');
            }
        }
    }
    out
}

/// One folder, as `listing` prints it.
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
pub(super) fn said(work: &FolderWork, address: &str) -> String {
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
