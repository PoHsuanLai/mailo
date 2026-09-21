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
pub struct Attachment {
    pub name: String,
    /// The declared media type. Untrusted; never used to decide how to execute anything.
    pub mime: String,
    pub size: u64,
    pub blob: BlobId,
    pub inline: Inline,
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
