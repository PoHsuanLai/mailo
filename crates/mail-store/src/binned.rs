//! What is in Trash and Spam, and since when (migration 0041): what erasing them after a while,
//! or at once under "Delete deletes it now", is decided from.

use chrono::{DateTime, Utc};
use mail_domain::{MailboxRole, MessageId, ThreadId};
use porter_core::AccountId;

/// One message in Trash or Spam, as [`crate::SqliteStore::binned`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binned {
    pub message: MessageId,
    pub thread: ThreadId,
    pub account: AccountId,
    /// Trash or Spam.
    pub mailbox: MailboxRole,
    /// When a sweep first found it there.
    pub since: DateTime<Utc>,
    /// Whether Delete put it there to be erased at once.
    pub soon: Soon,
    /// Whether the server has confirmed where it is: no change made here is still waiting to be
    /// sent, and the store holds its address on the server.
    pub settled: Settled,
}

/// Whether a message in Trash is to be erased as soon as the server holds it there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Soon {
    /// Delete put it there under "Delete deletes it now".
    Asked,
    /// It waits for its time like any other.
    Waits,
}

/// Whether a binned message is where the server says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// No change to it is waiting to be sent, and its address on the server is known.
    Yes,
    /// A move to it, or some other change, is still on its way: erasing now would be sent to
    /// where it was.
    Pending,
    /// No change to it is waiting, and the store holds no address for it: the server has not
    /// said where it put it yet, or its account has no server at all (local folders).
    Unplaced,
}
