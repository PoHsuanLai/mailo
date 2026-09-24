//! Answers to calendar invitations (iTIP, RFC 5546), as this client records them.
//!
//! The invitation itself is not domain state: it is read from the message's bytes each time it
//! is shown. What the user answered is, because nothing in those bytes says so — the answer
//! left in a message of its own — and the reader has to show it, and show the current one after
//! the user changes their mind.

use crate::MessageId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The three answers a person gives an invitation. `PARTSTAT` has more values, but the others
/// (`NEEDS-ACTION`, `DELEGATED`) are not something this client lets a person answer with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attendance {
    Accepted,
    Tentative,
    Declined,
}

/// What the user answered one invitation message, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteAnswer {
    /// The message holding the invitation that was answered.
    pub message: MessageId,
    pub attendance: Attendance,
    /// The invitation's `SEQUENCE` at the time: an answer to an older revision of an event is
    /// not an answer to its update.
    pub sequence: u32,
    /// The note sent with the answer, if any.
    pub comment: Option<String>,
    pub answered_at: DateTime<Utc>,
}
