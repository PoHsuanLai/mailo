//! Whether a message was written as rich text or as plain text, for a reply that answers in kind.

use crate::error::Logged;
use mail_domain::MessageId;
use mail_store::{SqliteStore, Store};

/// What a message's body was written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// It has an HTML part: formatting, with or without a plain copy beside it.
    Rich,
    /// Plain text alone.
    Plain,
}

/// What `message` was written as, read from its stored bytes. `None` when that cannot be said
/// here: the message is gone, its body has not been fetched, or its bytes do not parse. A caller
/// then falls back on what it would have done for a new message.
///
/// The bytes rather than the text stored beside them, because the stored text is there for every
/// message, rich or plain, and only the parse says whether there was an HTML part.
pub fn written_as(store: &SqliteStore, message: MessageId) -> Option<Written> {
    let original = store
        .message(message)
        .or_log("the message a reply answers could not be read")?;
    let raw = original.body.raw()?;
    let bytes = store
        .blobs()
        .get(raw)
        .or_log("the message a reply answers has no bytes here")?;
    // Bytes that do not parse say nothing about their format, and the reply is written as a new
    // message would be.
    let parsed = mail_mime::parse(&bytes).ok()?;
    Some(if parsed.html.is_some() {
        Written::Rich
    } else {
        Written::Plain
    })
}
