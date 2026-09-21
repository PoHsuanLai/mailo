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

/// A message body.
///
/// Holds the *raw* bytes and no sanitized HTML. Sanitizer output is not persisted: an
/// `ammonia` upgrade would otherwise leave every previously-ingested message sanitized under
/// the old rules. `mail-app` sanitizes at render time and caches by policy version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    /// The `text/plain` part, if the message had one. Used for search and for previews.
    pub text: Option<String>,
    /// The full raw message, content-addressed on disk. The single source of truth: every
    /// rendering, re-parse and re-sanitize starts here.
    pub raw: BlobId,
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
