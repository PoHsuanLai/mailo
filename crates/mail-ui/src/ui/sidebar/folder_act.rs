//! What a folder action does from the window: through `folder::change`, the one path the CLI
//! takes too, onto the undo stack, and into words.
use super::folder_tree::{AccountFolders, Mailboxes, placed};
use dioxus::prelude::*;
use mail_client::Client;
use mail_client::folder::Refusal;
use mail_client::query::Mailboxes as ClientMailboxes;
use mail_client::undo::{Undo, reverse_folder};
use mail_domain::{FolderError, FolderWork, LabelId, MailboxRef, NonEmpty, Subscription};
use mail_ui_kit::motion::record;
use mail_ui_kit::view::Shell;
use mail_ui_model::copy;
use porter_core::AccountId;

/// Every account in `scope` (empty meaning all), with what the section needs of each, from the
/// accounts the client keeps read.
pub(in crate::ui) fn load(
    accounts: &[mail_client::query::Account],
    scope: &mail_ui_kit::space::Scope,
) -> Vec<AccountFolders> {
    accounts
        .iter()
        .filter(|account| scope.shows(account.id.clone()))
        .map(|account| AccountFolders {
            address: mail_ui_kit::data::named(account),
            account: account.id.clone(),
            mailboxes: match &account.mailboxes {
                ClientMailboxes::One { .. } => Mailboxes::One,
                ClientMailboxes::Many {
                    folders,
                    labels,
                    server_labels,
                } => Mailboxes::Many {
                    folders: folders.clone(),
                    labels: labels.clone(),
                    server_labels: *server_labels,
                },
            },
        })
        .collect()
}

/// Every folder that is a place, across every account, named as its row is.
///
/// Every account and not the Space's: the places are the window's, and a Space narrows which
/// of them the sidebar draws.
pub(in crate::ui) fn folder_places(
    accounts: &[mail_client::query::Account],
) -> Vec<(String, MailboxRef)> {
    placed(&load(accounts, &mail_ui_kit::space::Scope::All))
}

/// The labels of every account in `scope`: the ones the Labels section draws.
///
/// The places hold every account's labels, as they hold every account's folders, so a label's
/// place keeps its index whatever the Space; the Space, or the pressed tile, narrows which of
/// them are drawn. Without it a Space whose accounts were all taken out still listed every other
/// account's labels.
pub(in crate::ui) fn labels_in(
    accounts: &[mail_client::query::Account],
    scope: &mail_ui_kit::space::Scope,
) -> Vec<LabelId> {
    accounts
        .iter()
        .filter(|account| scope.shows(account.id.clone()))
        .flat_map(|account| account.labels().iter().map(|label| label.id))
        .collect()
}

/// Every account's labels, by name: what the search box resolves `label:` against.
pub(in crate::ui) fn known_labels(
    accounts: &[mail_client::query::Account],
) -> Vec<(String, LabelId)> {
    accounts
        .iter()
        .flat_map(|account| {
            account
                .labels()
                .iter()
                .map(|label| (label.name.clone(), label.id))
        })
        .collect()
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
    client: &Client,
    account: AccountId,
    work: FolderWork,
    delimiter: Option<char>,
) -> Result<Done, Refusal> {
    let applied = client
        .folder()
        .change(account.clone(), work.clone(), chrono::Utc::now())?;
    let said = told(&work, delimiter);
    let undo = reverse_folder(&work).map(|_| Undo {
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
    client: &Client,
    shell: Signal<Shell>,
    account: AccountId,
    work: FolderWork,
    delimiter: Option<char>,
) -> Result<(), Refusal> {
    let done = perform(client, account, work, delimiter)?;
    record(shell, done.undo.into_iter().collect(), done.said);
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
/// is refused here, in words, before the client is asked.
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
