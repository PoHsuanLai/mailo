//! A message's MIME tree, as a server describes it before sending any of it.

use serde::{Deserialize, Serialize};

/// One node of a message's MIME structure: IMAP's `BODYSTRUCTURE`, reduced to what deciding
/// which parts to fetch and putting the message back together needs.
///
/// `section` is the IMAP section number the node is fetched by: `"2"`, `"1.3"`. The root of a
/// multipart message has the empty section, and its children are `"1"`, `"2"`, …; a message
/// that is not multipart is a single leaf with section `"1"`, whose headers are the message's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum PartTree {
    /// A part with content of its own. A `message/rfc822` part is one of these too: it is
    /// fetched or left whole, never taken apart.
    Leaf {
        section: String,
        /// `type/subtype`, lowercase. Declared by the sender, so untrusted.
        mime: String,
        /// The size on the wire, before any content transfer decoding.
        octets: u64,
        /// Declared `Content-Disposition: attachment`. A text part that is not one may be the
        /// message's body, so it is never left behind however large it is.
        attachment: bool,
    },
    /// `multipart/<subtype>`.
    Multipart {
        section: String,
        /// Lowercase: `mixed`, `alternative`, `related`.
        subtype: String,
        /// The delimiter between children, as the part's own `Content-Type` declares it.
        boundary: String,
        parts: Vec<PartTree>,
    },
}

impl PartTree {
    pub fn section(&self) -> &str {
        match self {
            PartTree::Leaf { section, .. } | PartTree::Multipart { section, .. } => section,
        }
    }
}
