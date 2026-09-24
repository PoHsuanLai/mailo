//! What a folder action does from the window: through `folder::change`, the one path the CLI
//! takes too, onto the undo stack, and into words.

use super::super::data::account_rows;
use super::super::motion::{Follow, tell};
use super::folder_tree::{AccountFolders, Mailboxes, placed};
use crate::folder::{Refusal, change};
use crate::undo::{Undo, reverse_folder};
use crate::view::Shell;
use dioxus::prelude::*;
use mail_domain::{
    AccountId, FolderError, FolderWork, Incoming, MailboxRef, NonEmpty, ServerLabels, Subscription,
};
use mail_store::{SqliteStore, Store};

/// Every account in `scope` (empty meaning all), with what the section needs of each.
///
/// Read with [`account_rows`], which tolerates a plan it cannot parse, so one odd account does
/// not take the section away from the others.
pub(in crate::ui) fn load(store: &SqliteStore, scope: &[AccountId]) -> Vec<AccountFolders> {
    account_rows(store)
        .into_iter()
        .filter(|row| scope.is_empty() || scope.contains(&row.id))
        .map(|row| {
            let mailboxes = match row.plan.incoming {
                // Local mail has no server folders to make either; its places are labels.
                Incoming::Pop3 { .. } | Incoming::Local => Mailboxes::One,
                Incoming::Imap { .. } | Incoming::Graph | Incoming::Jmap { .. } => {
                    Mailboxes::Many {
                        folders: store.folders(row.id).unwrap_or_default(),
                        labels: store.labels(row.id).unwrap_or_default(),
                        // Before the first connection nothing is known, and nothing is assumed.
                        server_labels: crate::sync::caps_of(store, row.id)
                            .map_or(ServerLabels::LocalOnly, |caps| caps.labels),
                    }
                }
            };
            AccountFolders {
                account: row.id,
                address: row.address,
                mailboxes,
            }
        })
        .collect()
}

/// Every folder that is a place, across every account, named as its row is.
///
/// Every account and not the Space's: the places are the window's, and a Space narrows which
/// of them the sidebar draws.
pub(in crate::ui) fn folder_places(store: &SqliteStore) -> Vec<(String, MailboxRef)> {
    placed(&load(store, &[]))
}

/// A change that happened: what the toast says, and how to take it back, when it can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Done {
    pub said: String,
    pub undo: Option<Undo>,
}

/// Make `work` on `account`: here at once, queued for the server, and kept for Undo.
///
/// The window's own function, and the one its tests drive. `delimiter` only shapes the words.
pub(in crate::ui) fn perform(
    store: &SqliteStore,
    account: AccountId,
    work: FolderWork,
    delimiter: Option<char>,
) -> Result<Done, Refusal> {
    let applied = change(store, account, work.clone(), chrono::Utc::now())?;
    let said = told(&work, delimiter);
    let undo = reverse_folder(&work).map(|_| Undo {
        said: said.clone(),
        thread: None,
        account,
        forward: applied.forward,
        inverse: applied.inverse,
        remote: applied.remote,
    });
    Ok(Done { said, undo })
}

/// [`perform`], then let the window show it: the undo on the stack, a fresh read of every
/// list, and the toast.
pub(in crate::ui) fn act(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
    account: AccountId,
    work: FolderWork,
    delimiter: Option<char>,
) -> Result<(), Refusal> {
    let done = perform(store, account, work, delimiter)?;
    let offer = match done.undo {
        Some(undo) => Follow::Undo(shell.write().undo.push(undo)),
        None => Follow::Nothing,
    };
    revision += 1;
    tell(done.said, offer);
    Ok(())
}

/// The last level of `path`: what a row calls it.
pub(in crate::ui) fn leaf(path: &str, delimiter: Option<char>) -> &str {
    delimiter
        .and_then(|d| path.rsplit_once(d))
        .map_or(path, |(_, name)| name)
}

/// What the toast says a folder change did.
pub(in crate::ui) fn told(work: &FolderWork, delimiter: Option<char>) -> String {
    let name = |path: &str| leaf(path, delimiter).to_owned();
    match work {
        FolderWork::Create { path } => format!("Folder “{}” made", name(path)),
        FolderWork::Rename { from, to } => {
            format!("“{}” renamed to “{}”", name(from), name(to))
        }
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Refuse,
        } => format!("Folder “{}” deleted", name(path)),
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Allow,
        } => format!("Folder “{}” deleted with its mail", name(path)),
        FolderWork::Subscribe {
            path,
            subscription: Subscription::Subscribed,
        } => format!("Following “{}”", name(path)),
        FolderWork::Subscribe { path, .. } => format!("Stopped following “{}”", name(path)),
    }
}

/// Why a change did not happen, in the window's words.
///
/// A folder holding mail is not here: that one is a question, asked in the menu.
pub(in crate::ui) fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Folder(error) => match error {
            FolderError::SingleMailbox => {
                "A POP3 account has one mailbox, and no folders to make.".to_owned()
            }
            FolderError::KeptLocally => {
                "This mail is kept on this computer: it has labels, not server folders.".to_owned()
            }
            FolderError::Special { path, special } => format!(
                "“{path}” is the account's {} folder. Other mail apps depend on it, so it stays as it is.",
                special.name()
            ),
            FolderError::NotEmpty { path, messages } => {
                format!("“{path}” holds {} and was kept.", messages_word(*messages))
            }
            FolderError::Exists(name) => format!("There is already a folder called “{name}”."),
            FolderError::Unknown(name) => {
                format!("“{name}” is not on the server's list any more. Sync to refresh it.")
            }
            FolderError::HasChildren(name) => {
                format!("“{name}” has folders inside it. Move or delete those first.")
            }
            FolderError::BadName { name, why } => {
                format!("“{name}” cannot be a folder name: {why}.")
            }
        },
        Refusal::NoAccount => "That account is not set up here any more.".to_owned(),
        Refusal::Store(why) => format!("Nothing was changed: {why}"),
    }
}

/// "1 message", "132 messages".
pub(in crate::ui) fn messages_word(count: u64) -> String {
    if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{count} messages")
    }
}

/// The path a new name makes: inside `parent`, or at the top.
///
/// A name is one level. Typing the separator into it would make levels nobody asked for, so it
/// is refused here, in words, before the store is asked.
pub(in crate::ui) fn child_path(
    parent: Option<&str>,
    name: &str,
    delimiter: Option<char>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name first.".to_owned());
    }
    if let Some(d) = delimiter
        && name.contains(d)
    {
        return Err(format!(
            "A folder name cannot contain “{d}”: it separates folders."
        ));
    }
    match (parent, delimiter) {
        (None, _) => Ok(name.to_owned()),
        (Some(parent), Some(d)) => Ok(format!("{parent}{d}{name}")),
        (Some(_), None) => Err("This server keeps every folder at the top level.".to_owned()),
    }
}

/// The path `path` has once its last level is called `name`.
pub(in crate::ui) fn renamed_path(
    path: &str,
    name: &str,
    delimiter: Option<char>,
) -> Result<String, String> {
    let parent = delimiter
        .and_then(|d| path.rsplit_once(d))
        .map(|(parent, _)| parent);
    child_path(parent, name, delimiter)
}
