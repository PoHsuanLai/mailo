//! How local objects relate to what is on a server, and what we ask a server to do.
//!
//! These types live in the domain rather than in a protocol crate because `mail-store`
//! persists them — `remote_map`, `sync_state` and `outbox` — and `mail-store` does not depend
//! on `mail-proto`. They are protocol-neutral vocabulary; wire *syntax* stays in `mail-proto`.

use crate::id::{AccountId, BlobId, DraftId};
use crate::state::MailboxRole;
use serde::{Deserialize, Serialize};

/// One server-side mailbox. `"INBOX"` for POP3, which has only one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailboxRef {
    pub account: AccountId,
    /// The IMAP folder path exactly as the server spells it, e.g. `"[Gmail]/All Mail"`.
    pub path: String,
}

/// Where a message sits on a server.
///
/// **This is many-to-one with [`crate::MessageId`], not a bijection.** On Gmail the same
/// message exists in `INBOX` and `[Gmail]/All Mail` under *different* UIDs, and again in
/// `[Gmail]/Sent` if you sent it. Identity comes from [`crate::MessageKey`]; a `RemoteRef` is
/// only an address. Treating it as an identity duplicates every message on the first sync.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum RemoteRef {
    Imap {
        mailbox: String,
        /// Scopes `uid`. When the server reports a different value, every `uid` for this
        /// mailbox is meaningless and the mapping must be rebuilt.
        uidvalidity: u32,
        uid: u32,
    },
    Pop {
        uidl: String,
    },
}

/// How far a mailbox has been synced. Per mailbox, not per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SyncCursor {
    Imap {
        uidvalidity: u32,
        uidnext: u32,
        /// `HIGHESTMODSEQ`, when the server offers `CONDSTORE`. Enables incremental flag
        /// sync instead of refetching every flag on every poll.
        modseq: Option<u64>,
    },
    /// POP3 keeps no cursor: every poll lists `UIDL` in full and diffs against `remote_map`.
    Pop,
}

/// Whether the server's `UIDVALIDITY` still matches what we stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UidValidity {
    Same,
    /// Every `remote_map` row for this mailbox must be dropped and the mailbox refetched.
    Reset,
}

/// Where to resume a fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum FetchSince {
    Beginning,
    After { cursor: SyncCursor },
}

/// A unit of remote work. Queued in the outbox, retried with backoff, drained serially per
/// account so that two operations on one thread have a defined result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum ProtoOp {
    /// Refresh [`crate::AccountCaps`] from `CAPABILITY` / `CAPA`.
    FetchCaps,
    /// `LIST` the server's folders, to rebuild [`crate::FolderRoles`].
    ListFolders,
    FetchEnvelopes {
        mailbox: MailboxRef,
        since: FetchSince,
    },
    FetchBody {
        remote: RemoteRef,
    },
    SetFlags {
        remotes: Vec<RemoteRef>,
        read: Option<crate::state::ReadState>,
        star: Option<crate::state::Star>,
    },
    SetMailbox {
        remotes: Vec<RemoteRef>,
        role: MailboxRole,
    },
    SetLabels {
        remotes: Vec<RemoteRef>,
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// Upload a message we composed, e.g. a draft to the Drafts folder.
    Append {
        mailbox: MailboxRef,
        raw: BlobId,
        role: MailboxRole,
    },
    Submit {
        draft: DraftId,
        raw: BlobId,
    },
    Expunge {
        remotes: Vec<RemoteRef>,
    },
    Watch {
        mailbox: MailboxRef,
    },
}
