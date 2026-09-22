//! Message content: who, what, and the bytes behind it.

use crate::id::{AccountId, BlobId, LabelId};
use crate::state::LabelOrigin;
use serde::{Deserialize, Serialize};

/// A mailbox address with its optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

/// A message body, which may not have been fetched yet.
///
/// Holds the *raw* bytes and no sanitized HTML. Sanitizer output is not persisted: an
/// `ammonia` upgrade would otherwise leave every previously-ingested message sanitized under
/// the old rules. `mail-app` sanitizes at render time and caches by policy version.
///
/// An enum rather than a struct with an optional `raw`, because **headers-without-body is a
/// normal state, not an error**. A POP3 first sync fetches headers with `TOP` before any body:
/// `RETR` marks a message read on the server, so retrieving 2372 messages to populate a list
/// view would mark the user's entire mailbox read in their webmail. IMAP does the same thing
/// for a different reason — envelopes first, bodies on demand.
///
/// Making it an `Option<BlobId>` would let a caller reach for `raw` and get `None` without
/// having thought about which case it is in; this way the compiler asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Body {
    /// Headers fetched, body not yet retrieved.
    Absent,
    Present {
        /// The `text/plain` part, if the message had one. Used for search and for previews.
        text: Option<String>,
        /// The full raw message, content-addressed on disk. The single source of truth: every
        /// rendering, re-parse and re-sanitize starts here.
        ///
        /// For a large IMAP message this is the message *as rebuilt from its parts*: every
        /// header and every text part exactly as sent, and each attachment left on the server
        /// present with its own headers and an empty body. Such an attachment is
        /// [`PartContent::Remote`] in [`crate::Message::attachments`], which is what says so.
        raw: BlobId,
    },
}

impl Body {
    /// The raw blob, or `None` while only headers are held.
    pub fn raw(&self) -> Option<BlobId> {
        match self {
            Body::Absent => None,
            Body::Present { raw, .. } => Some(*raw),
        }
    }

    /// The plain-text part, or `None` when absent or not yet fetched.
    pub fn text(&self) -> Option<&str> {
        match self {
            Body::Absent => None,
            Body::Present { text, .. } => text.as_deref(),
        }
    }
}

/// Whether a part is offered as a download or referenced from the HTML body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Inline {
    Attached,
    /// Referenced by `cid:` from the HTML body. The `cid` is untrusted input from the
    /// message and is only ever matched against this list — never used to build a path.
    Embedded {
        cid: String,
    },
}

/// One MIME part worth showing to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "AttachmentRow", into = "AttachmentRow")]
pub struct Attachment {
    pub name: String,
    /// The declared media type. Untrusted; never used to decide how to execute anything.
    pub mime: String,
    /// Decoded bytes when held; the server's figure for a part still on it, which for base64 is
    /// about a third more than the file will be.
    pub size: u64,
    pub content: PartContent,
    pub inline: Inline,
}

impl Attachment {
    /// The stored bytes, or `None` while the part is still on the server.
    pub fn blob(&self) -> Option<BlobId> {
        match self.content {
            PartContent::Held(blob) => Some(blob),
            PartContent::Remote { .. } => None,
        }
    }
}

/// Where an attachment's bytes are.
///
/// An enum for the reason [`Body`] is one: a part not yet downloaded is a normal state, and a
/// caller reaching for bytes should have to say what it does when there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartContent {
    /// Stored locally.
    Held(BlobId),
    /// Still on the server, as this IMAP section of the message (`"2"`, `"1.3"`). Fetched when
    /// it is opened or saved, never by a sync.
    Remote { section: String },
}

/// [`Attachment`] as a stored JSON row.
///
/// Rows written before parts could be remote have `blob` and nothing else, and they must still
/// read: this is the one shape both kinds of row fit. `blob` wins when a row somehow has both.
#[derive(Serialize, Deserialize)]
struct AttachmentRow {
    name: String,
    mime: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<BlobId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_section: Option<String>,
    inline: Inline,
}

impl From<AttachmentRow> for Attachment {
    fn from(row: AttachmentRow) -> Self {
        let content = match (row.blob, row.remote_section) {
            (Some(blob), _) => PartContent::Held(blob),
            (None, Some(section)) => PartContent::Remote { section },
            // Neither: not a row this code ever wrote. An empty section fetches nothing and
            // fails loudly when asked to, which beats inventing a blob id that names nothing.
            (None, None) => PartContent::Remote {
                section: String::new(),
            },
        };
        Attachment {
            name: row.name,
            mime: row.mime,
            size: row.size,
            content,
            inline: row.inline,
        }
    }
}

impl From<Attachment> for AttachmentRow {
    fn from(a: Attachment) -> Self {
        let (blob, remote_section) = match a.content {
            PartContent::Held(blob) => (Some(blob), None),
            PartContent::Remote { section } => (None, Some(section)),
        };
        AttachmentRow {
            name: a.name,
            mime: a.mime,
            size: a.size,
            blob,
            remote_section,
            inline: a.inline,
        }
    }
}

/// A flat label. Nested labels are a non-goal in v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: LabelId,
    pub account: AccountId,
    pub name: String,
    /// `#rrggbb`, or `None` to let the UI choose.
    pub color: Option<String>,
    pub origin: LabelOrigin,
}
