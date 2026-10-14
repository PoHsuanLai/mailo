//! What a folder action does from the window: through `folder::change`, the one path the CLI
//! takes too, onto the undo stack, and into words.

use super::super::data::account_rows;
use super::super::motion::{Follow, tell};
use super::folder_tree::{AccountFolders, Mailboxes, placed};
use crate::ui::copy;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use mail_core::folder::{Refusal, change};
use mail_core::undo::{Undo, reverse_folder};
use mail_core::{SqliteStore, Store};
use mail_domain::{
    FolderError, FolderWork, Incoming, MailboxRef, NonEmpty, ServerLabels, Subscription,
};
use porter_core::AccountId;

/// Every account in `scope` (empty meaning all), with what the section needs of each.
///
/// Read with [`account_rows`], which tolerates a plan it cannot parse, so one odd account does
/// not take the section away from the others.
pub(in crate::ui) fn load(
    store: &SqliteStore,
    scope: &crate::ui::space::Scope,
) -> Vec<AccountFolders> {
    account_rows(store)
        .into_iter()
        .filter(|row| scope.shows(row.id.clone()))
        .map(|row| {
            let mailboxes = match row.plan.incoming {
                // Local mail has no server folders to make either; its places are labels.
                Incoming::Pop3 { .. } | Incoming::Local => Mailboxes::One,
                Incoming::Imap { .. } | Incoming::Graph | Incoming::Jmap { .. } => {
                    Mailboxes::Many {
                        folders: store.folders(row.id.clone()).unwrap_or_default(),
                        labels: store.labels(row.id.clone()).unwrap_or_default(),
                        // Before the first connection nothing is known, and nothing is assumed.
                        server_labels: mail_core::sync::caps_of(store, row.id.clone())
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
    placed(&load(store, &crate::ui::space::Scope::All))
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
    let applied = change(store, account.clone(), work.clone(), chrono::Utc::now())?;
    let said = told(&work, delimiter);
    let undo = reverse_folder(&work).map(|_| Undo {
        said: said.clone(),
        thread: None,
        account,
        forward: applied.forward,
        inverse: applied.inverse,
        remote: applied.remote,
        resend: None,
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
        FolderWork::Create { path } => copy::toasts::folder_made(&name(path)),
        FolderWork::Rename { from, to } => copy::toasts::folder_renamed(&name(from), &name(to)),
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Refuse,
        } => copy::toasts::folder_deleted(&name(path)),
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Allow,
        } => copy::toasts::folder_deleted_with_mail(&name(path)),
        FolderWork::Subscribe {
            path,
            subscription: Subscription::Subscribed,
        } => copy::toasts::following(&name(path)),
        FolderWork::Subscribe { path, .. } => copy::toasts::stopped_following(&name(path)),
    }
}

/// Why a change did not happen, in the window's words.
///
/// A folder holding mail is not here: that one is a question, asked in the menu.
pub(in crate::ui) fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Folder(error) => match error {
            FolderError::SingleMailbox => copy::errors::POP3_NO_FOLDERS.to_owned(),
            FolderError::KeptLocally => copy::errors::LOCAL_HAS_LABELS.to_owned(),
            FolderError::Special { path, special } => {
                copy::errors::special_folder(path, special.name())
            }
            FolderError::NotEmpty { path, messages } => {
                copy::errors::folder_holds(path, &copy::status::messages(*messages))
            }
            FolderError::Exists(name) => copy::errors::folder_exists(name),
            FolderError::Unknown(name) => copy::errors::folder_unknown(name),
            FolderError::HasChildren(name) => copy::errors::folder_has_children(name),
            FolderError::BadName { name, why } => copy::errors::bad_folder_name(name, why),
        },
        Refusal::NoAccount => copy::errors::ACCOUNT_GONE.to_owned(),
        Refusal::Store(why) => copy::errors::nothing_changed(why),
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
        return Err(copy::errors::TYPE_A_NAME.to_owned());
    }
    if let Some(d) = delimiter
        && name.contains(d)
    {
        return Err(copy::errors::name_has_delimiter(d));
    }
    // A name spelled as a system flag is the server's, and is never drawn (`Folder::named`).
    if name.starts_with('\\') {
        return Err(copy::errors::NAME_STARTS_WITH_FLAG.to_owned());
    }
    match (parent, delimiter) {
        (None, _) => Ok(name.to_owned()),
        (Some(parent), Some(d)) => Ok(format!("{parent}{d}{name}")),
        (Some(_), None) => Err(copy::errors::NO_SUBFOLDERS.to_owned()),
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
