//! Mailboxes as the user manages them: what a server lists, and asking it to change the list.
//!
//! A folder here is one IMAP mailbox, whatever it is used for. [`crate::MailboxRole`] is the
//! small closed set of places a *message* can be filed; a [`Folder`] is any name the server
//! lists, which on Gmail includes every label, because Gmail presents each label to IMAP as a
//! mailbox. Creating, renaming and deleting are therefore one vocabulary for both.
//!
//! The work itself ([`FolderWork`]) is an outbox operation like a flag change: applied locally
//! at once, queued, sent in order, and undone if the server refuses it for good. [`plan`] is
//! where the local half and its undo are decided, and where the refusals that need no server
//! live.

mod plan;

pub use plan::{FolderContents, FolderCtx, plan};

use crate::id::AccountId;
use crate::retry::{Retry, Retryable};
use serde::{Deserialize, Serialize};

/// One mailbox on a server, as the last listing (and anything since queued) describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub account: AccountId,
    /// The full path, decoded from modified UTF-7: `"Projects/2026"`, `"[Gmail]/Sent Mail"`,
    /// `"收件匣"`. Encoded again only on the wire, the way [`crate::MailboxRef::path`] is.
    pub path: String,
    /// The hierarchy separator the server reported for this name, or `None` for a server with
    /// no hierarchy (`NIL` in `LIST`).
    pub delimiter: Option<char>,
    /// What the server says this mailbox is for, if anything.
    pub special: Option<SpecialUse>,
    pub subscription: Subscription,
    pub holds: Holds,
}

impl Folder {
    /// Why this mailbox may not be renamed or deleted, if it may not.
    ///
    /// `INBOX` by name as well as by attribute: RFC 3501 gives it that name on every server,
    /// whether or not the server marks it, and it is the one mailbox that cannot go.
    pub fn protected(&self) -> Option<SpecialUse> {
        self.special
            .or_else(|| is_inbox(&self.path).then_some(SpecialUse::Inbox))
    }

    /// Whether `path` is this folder or lies anywhere beneath it.
    pub fn contains(&self, path: &str) -> bool {
        path == self.path || is_beneath(path, &self.path, self.delimiter)
    }
}

/// The special uses a server can give a mailbox (RFC 6154, and Gmail's `\Important`), plus the
/// inbox, which RFC 3501 names rather than marks.
///
/// Distinct from [`crate::MailboxRole`] on purpose. A role is where this client files a
/// message; a special use is what the server says a mailbox is, and there are more of them —
/// `\Flagged` and `\Important` are views, not places anything is filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialUse {
    Inbox,
    /// `\All`: every message, as on Gmail's All Mail.
    All,
    Archive,
    Drafts,
    Flagged,
    Important,
    Junk,
    Sent,
    Trash,
}

impl SpecialUse {
    /// The name a person knows it by.
    pub fn name(self) -> &'static str {
        match self {
            SpecialUse::Inbox => "Inbox",
            SpecialUse::All => "All Mail",
            SpecialUse::Archive => "Archive",
            SpecialUse::Drafts => "Drafts",
            SpecialUse::Flagged => "Starred",
            SpecialUse::Important => "Important",
            SpecialUse::Junk => "Junk",
            SpecialUse::Sent => "Sent",
            SpecialUse::Trash => "Trash",
        }
    }
}

/// Whether the server lists a mailbox among the ones the user follows (`LSUB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subscription {
    Subscribed,
    Unsubscribed,
}

/// Whether a mailbox can hold mail, or only other mailboxes (`\Noselect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holds {
    Mail,
    /// A name that exists only to hold others, such as Gmail's `[Gmail]`.
    FoldersOnly,
}

/// What deleting a mailbox that still holds mail should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonEmpty {
    /// Refuse. Checked here against what this client holds, and again by the server at the
    /// moment of deletion, because a folder this client has never synced looks empty from here.
    Refuse,
    /// Delete it with its mail. Where mailboxes are labels this removes the label from each
    /// message and deletes nothing else.
    Allow,
}

/// A change to the server's set of mailboxes. Persisted in the outbox as part of
/// [`crate::ProtoOp::Folder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum FolderWork {
    /// `CREATE`, then `SUBSCRIBE`, so the new folder shows in every client that lists only
    /// what the user follows.
    Create { path: String },
    /// `RENAME`. Everything beneath `from` moves with it, on the server and here.
    Rename { from: String, to: String },
    /// `DELETE`.
    Delete { path: String, non_empty: NonEmpty },
    /// `SUBSCRIBE` or `UNSUBSCRIBE`.
    Subscribe {
        path: String,
        subscription: Subscription,
    },
}

impl FolderWork {
    /// The mailbox this work names first: the one created, renamed, deleted or followed.
    pub fn path(&self) -> &str {
        match self {
            FolderWork::Create { path }
            | FolderWork::Delete { path, .. }
            | FolderWork::Subscribe { path, .. } => path,
            FolderWork::Rename { from, .. } => from,
        }
    }
}

/// Why a folder change was refused before anything was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// POP3 has one mailbox, and nothing to name.
    #[error(
        "a POP3 account has exactly one mailbox; there are no folders to create, rename or delete"
    )]
    SingleMailbox,
    /// Mail kept only on this computer has no server to hold folders; its places are labels.
    #[error("this account's mail is kept on this computer; it has no server folders, only labels")]
    KeptLocally,
    /// A special-use mailbox, or the inbox, or a folder holding one.
    #[error("{path} is the account's {} folder; the server and other clients depend on it", special.name())]
    Special { path: String, special: SpecialUse },
    #[error("{path} holds {messages} message(s); say so explicitly to delete it with them")]
    NotEmpty { path: String, messages: u64 },
    #[error("there is already a folder called {0}")]
    Exists(String),
    #[error("there is no folder called {0}; `mailo sync` refreshes the list")]
    Unknown(String),
    #[error("{0} has folders inside it; rename or delete those first")]
    HasChildren(String),
    #[error("{name:?} cannot be a folder name: {why}")]
    BadName { name: String, why: String },
}

impl Retryable for FolderError {
    fn retry(&self) -> Retry {
        // Every one of these is a fact about the request, not the moment.
        Retry::Fatal(self.to_string())
    }
}

/// `path` with `from` replaced by `to`, when `path` is `from` or lies beneath it.
///
/// What a rename does to every name it touches: the folder itself, the folders inside it, the
/// server addresses of their messages and, on Gmail, their labels. Beneath means "after the
/// delimiter", never "starts with": renaming `Work` must not touch `Workshop`.
pub fn renamed(path: &str, from: &str, to: &str, delimiter: Option<char>) -> Option<String> {
    if path == from {
        return Some(to.to_owned());
    }
    let d = delimiter?;
    let rest = path.strip_prefix(from)?.strip_prefix(d)?;
    Some(format!("{to}{d}{rest}"))
}

/// Whether `path` lies strictly beneath `ancestor`.
fn is_beneath(path: &str, ancestor: &str, delimiter: Option<char>) -> bool {
    delimiter.is_some_and(|d| {
        path.strip_prefix(ancestor)
            .and_then(|rest| rest.strip_prefix(d))
            .is_some_and(|rest| !rest.is_empty())
    })
}

/// `INBOX` in any case, which RFC 3501 makes one name.
fn is_inbox(path: &str) -> bool {
    path.eq_ignore_ascii_case("INBOX")
}

/// Whether two paths name the same mailbox: exact, except that `INBOX` is case-insensitive.
fn same(a: &str, b: &str) -> bool {
    a == b || (is_inbox(a) && is_inbox(b))
}

/// The hierarchy separator a new name on this account will be read with.
///
/// One per account in practice (one per namespace in the RFC, and personal folders are one
/// namespace), so the first one the listing reports is the answer. `None` for a server with no
/// hierarchy, or one never listed.
pub fn delimiter_of(folders: &[Folder]) -> Option<char> {
    folders.iter().find_map(|f| f.delimiter)
}

/// A listing with queued work laid over it.
///
/// A fresh `LIST` can arrive between a folder being created here and the server being told,
/// and it will not have the folder in it. Replacing what is held with it outright would make
/// the folder vanish until the outbox drains, and then reappear — the folder equivalent of a
/// star flickering off under the cursor. So the listing is the base and the unconfirmed work
/// goes back on top, in the order it was queued, as `pending_changes` does for messages.
pub fn layered(account: AccountId, listed: Vec<Folder>, pending: &[FolderWork]) -> Vec<Folder> {
    let mut folders = listed;
    for work in pending {
        match work {
            FolderWork::Create { path } => {
                if !folders.iter().any(|f| same(&f.path, path)) {
                    let fresh = created(account, path, &folders);
                    folders.push(fresh);
                }
            }
            FolderWork::Rename { from, to } => {
                let delimiter = folders
                    .iter()
                    .find(|f| f.path == *from)
                    .and_then(|f| f.delimiter);
                for folder in &mut folders {
                    if let Some(path) = renamed(&folder.path, from, to, delimiter) {
                        folder.path = path;
                    }
                }
            }
            FolderWork::Delete { path, .. } => folders.retain(|f| f.path != *path),
            FolderWork::Subscribe { path, subscription } => {
                for folder in folders.iter_mut().filter(|f| f.path == *path) {
                    folder.subscription = *subscription;
                }
            }
        }
    }
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    folders
}

/// The folder `CREATE` makes, as this client records it before the server has said so.
fn created(account: AccountId, path: &str, folders: &[Folder]) -> Folder {
    Folder {
        account,
        path: path.to_owned(),
        delimiter: delimiter_of(folders),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_moves_what_is_beneath_and_nothing_that_merely_shares_a_prefix() {
        const CASES: &[(&str, Option<&str>)] = &[
            ("Work", Some("Jobs")),
            ("Work/2026", Some("Jobs/2026")),
            ("Work/2026/Q1", Some("Jobs/2026/Q1")),
            ("Workshop", None),
            ("Work.2026", None),
            ("Other", None),
        ];
        for (path, want) in CASES {
            assert_eq!(
                renamed(path, "Work", "Jobs", Some('/')).as_deref(),
                *want,
                "{path}"
            );
        }
    }

    #[test]
    fn without_a_hierarchy_only_the_name_itself_is_renamed() {
        assert_eq!(
            renamed("Work", "Work", "Jobs", None).as_deref(),
            Some("Jobs")
        );
        assert_eq!(renamed("Work/x", "Work", "Jobs", None), None);
    }

    #[test]
    fn the_inbox_is_protected_by_name_even_when_the_server_does_not_mark_it() {
        let folder = Folder {
            account: AccountId::from_uuid(uuid::Uuid::nil()),
            path: "inbox".to_owned(),
            delimiter: Some('/'),
            special: None,
            subscription: Subscription::Subscribed,
            holds: Holds::Mail,
        };
        assert_eq!(folder.protected(), Some(SpecialUse::Inbox));
    }
}
