//! What the store knows of the people who write to the user.

use chrono::{DateTime, Utc};

/// One sender, over every message the store holds from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderRecord {
    /// Lower-cased, which is how senders are told apart.
    pub email: String,
    /// The name on their newest message, as written.
    pub name: Option<String>,
    /// In how many conversations they have written.
    pub threads: u32,
    /// When their newest message is dated.
    pub last: DateTime<Utc>,
    /// Whether any of the user's accounts has written to this address.
    pub replied: bool,
}
