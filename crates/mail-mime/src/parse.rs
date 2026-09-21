//! RFC 5322 / MIME bytes to domain values.

use crate::MimeError;
use chrono::{DateTime, Utc};
use mail_domain::{Address, Inline};

/// One part worth keeping: an attachment, or an image the HTML body references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPart {
    pub name: String,
    /// The declared media type, untrusted. Never used to decide how to execute anything.
    pub mime: String,
    pub bytes: Vec<u8>,
    pub inline: Inline,
}

/// Everything a message's bytes say about it.
///
/// Deliberately *not* a [`mail_domain::Message`]: that needs a `MessageId`, `ThreadId`,
/// `AccountId` and a stored `BlobId`, none of which the bytes can supply. The caller assigns
/// those and assembles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The `Message-ID`, normalized by [`mail_domain::normalize_id`]. `None` when absent or
    /// unusable, in which case the caller must fall back to [`mail_domain::MessageKey::Synthetic`].
    pub rfc_message_id: Option<String>,
    /// `None` when the `Date` header is absent or unparseable — common, and not an error.
    /// The caller substitutes the server's internal date.
    pub date: Option<DateTime<Utc>>,
    pub from: Option<Address>,
    /// Empty when absent. A reply goes here in preference to `from`.
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// Normalized like `rfc_message_id`.
    pub in_reply_to: Option<String>,
    /// Normalized, oldest first, duplicates removed.
    pub references: Vec<String>,
    pub text: Option<String>,
    /// Raw, unsanitized. Sanitization happens at render time; see [`crate::sanitize`].
    pub html: Option<String>,
    pub attachments: Vec<ParsedPart>,
}

/// Parse a whole RFC 5322 message.
///
/// Must not fail on mail that is merely malformed — a broken `Date`, a missing `Message-ID`,
/// a mislabelled charset, an 8-bit header. Those are facts to record, not errors. Reserve
/// [`MimeError::Unparseable`] for bytes that are not a message at all.
pub fn parse(_raw: &[u8]) -> Result<Parsed, MimeError> {
    todo!("wave 2")
}
