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
    pub labels: Vec<Label>,
    /// Expunged on the server, by this client or another one. Without this, a message
    /// deleted elsewhere never disappears locally.
    pub gone: Vec<RemoteRef>,
}
