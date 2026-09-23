//! Read receipts: message disposition notifications (RFC 8098, which replaced RFC 3798).
//!
//! Three separate facts, kept as three types because they belong to three different objects:
//! whether a draft *asks* for one ([`ReceiptRequest`], on the draft), what this client did about
//! a message that asked ([`ReceiptAnswer`], recorded per message so it is asked once), and the
//! server-side keyword that tells every other client the same thing ([`Keyword::MdnSent`]).
//!
//! Nothing here sends anything. RFC 8098 §2.1 is explicit that a receipt must not go out
//! without the user's say-so, and the only path that builds one is a command the user runs.

use serde::{Deserialize, Serialize};

/// Whether a draft asks its recipients to confirm they have displayed it.
///
/// Written as `Disposition-Notification-To` naming the sender. Most recipients' clients will
/// ask their user first, and many never answer: a receipt that arrives is evidence, one that
/// does not is evidence of nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptRequest {
    /// The default, and what every draft stored before receipts existed deserializes as.
    #[default]
    Unrequested,
    Requested,
}

/// What the user did when a message asked for a receipt.
///
/// Either answer settles it: RFC 3503 sets `$MDNSent` for a receipt sent *and* for one the
/// user declined, because in both cases the question has been put and must not be put again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptAnswer {
    Sent,
    Declined,
}

/// An IMAP keyword this client sets on a message. Closed: a keyword is a flag other clients
/// read, and one this client invents on the fly is noise in every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keyword {
    /// RFC 3503 `$MDNSent`: a receipt for this message was sent or declined.
    MdnSent,
}
