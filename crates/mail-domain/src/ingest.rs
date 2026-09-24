//! Bulk facts arriving from a server. The counterpart to [`crate::Patch`].

use crate::content::Label;
use crate::id::BlobId;
use crate::message::{Message, MessageKey};
use crate::remote::{MailboxRef, RemoteRef, SyncCursor, UidValidity};
use crate::state::{ReadState, Star};
use serde::{Deserialize, Serialize};

/// One message as the server just gave it to us.
///
/// Carries a fully-parsed domain [`Message`]: parsing happens in `mail-proto` via `mail-mime`,
/// so this crate never needs to name a MIME type and there is no dependency cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    pub remote: RemoteRef,
    pub key: MessageKey,
    /// The raw bytes, already stored. Everything else here can be rebuilt from it.
    pub raw: BlobId,
    pub message: Message,
}

/// The result of one sync pass over one mailbox.
///
/// Not a [`crate::Patch`]: this is bulk, not invertible, and it is server truth rather than an
/// optimistic guess. `mail-store` writes it, re-layers any still-pending local changes on top,
/// and returns a `Patch` describing what actually moved so the UI can refresh precisely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ingest {
    pub mailbox: MailboxRef,
    /// [`UidValidity::Reset`] invalidates every `remote_map` row for this mailbox.
    pub validity: UidValidity,
    /// Where this ingest leaves the mailbox, or `None` when it says nothing about that.
    ///
    /// `None` is not a missing value, it is a claim: *this batch learned nothing about the
    /// mailbox's position*. A header or body fetch is exactly that — it was told which messages
    /// to collect and collected them, and it never asked the server what exists or how far the
    /// mailbox has moved. Only a survey can answer that.
    ///
    /// It was a plain `SyncCursor` and the runtime passed `SyncCursor::Pop` for every header
    /// fetch on every protocol, which overwrote the real IMAP cursor a few milliseconds after
    /// the survey wrote it. `UIDVALIDITY`, `UIDNEXT` and `HIGHESTMODSEQ` were destroyed on every
    /// pass, so an IMAP account resurveyed its whole mailbox for ever and the CONDSTORE path
    /// could never start.
    pub cursor: Option<SyncCursor>,
    /// New or refetched messages.
    pub messages: Vec<Fetched>,
    /// Flag-only updates, which are far cheaper than refetching a message.
    pub flags: Vec<(RemoteRef, ReadState, Star)>,
    /// Label definitions this batch learned about.
    pub labels: Vec<Label>,
    /// What the server says each message is labelled, by name.
    ///
    /// Names and not [`LabelId`]s because the protocol has names and the store owns ids: the
    /// store creates any it has not seen, with [`LabelOrigin::Provider`]. Parallel to `flags`
    /// and for the same reason — a label change is far cheaper than refetching a message.
    ///
    /// The list is **complete**, like `flags`: it is what the message is labelled *now*, not
    /// what was added. A label the server no longer lists has been removed there, and a client
    /// that only ever adds accumulates labels the user deleted years ago.
    ///
    /// Empty on every protocol but Gmail, which is the only one that has them.
    #[serde(default)]
    pub label_names: Vec<(RemoteRef, Vec<String>)>,
    /// Expunged on the server, by this client or another one. Without this, a message
    /// deleted elsewhere never disappears locally.
    pub gone: Vec<RemoteRef>,
}

/// One message kept by this client alone, with nothing on any server to point at.
///
/// What [`Fetched`] would be without a [`RemoteRef`]: imported mail in the local-only account, or
/// a message just uploaded to a server that did not say what UID it got. Absorbing one writes no
/// `remote_map` row, so nothing done to it later is ever queued for a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub key: MessageKey,
    /// The raw bytes, already stored.
    pub raw: BlobId,
    pub message: Message,
    /// Labels by name, created on the account where new. Added to a message already held,
    /// never taken away: the same message found in two folders of an export is one message
    /// with both labels.
    #[serde(default)]
    pub labels: Vec<String>,
}

/// A batch of [`Kept`] messages, deduplicated by [`MessageKey`] like an [`Ingest`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Import {
    pub messages: Vec<Kept>,
}
