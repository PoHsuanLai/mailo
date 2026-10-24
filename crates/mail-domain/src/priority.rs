//! How urgent a message says it is.
//!
//! Advisory, and only that: no server orders delivery by it. The recipient's client may draw a
//! flag beside the message, and many draw nothing at all.

use serde::{Deserialize, Serialize};

/// A message's stated urgency, written as `X-Priority` and `Importance` when it is not
/// [`Priority::Normal`].
///
/// Both headers, because clients read one or the other: Outlook and Exchange read `Importance`
/// (RFC 2156 §5.3), and most of the rest read the older `X-Priority`. Normal writes neither, so
/// almost every message carries no claim about itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    High,
    /// The default, and what every draft stored before priorities existed deserializes as.
    #[default]
    Normal,
    Low,
}

impl Priority {
    /// Every priority, most urgent first, as a menu lists them.
    pub const ALL: [Priority; 3] = [Priority::High, Priority::Normal, Priority::Low];

    /// `X-Priority` and `Importance` for this priority, or `None` for Normal, which says
    /// nothing. `X-Priority` is the bare digit on its 1-to-5 scale, high at 1 and low at 5.
    pub fn headers(self) -> Option<(&'static str, &'static str)> {
        match self {
            Priority::High => Some(("1", "high")),
            Priority::Normal => None,
            Priority::Low => Some(("5", "low")),
        }
    }
}
