//! What the reader needs from the store that is not already a column.
//!
//! One function so far, and it exists because the HTML part of a message is not stored
//! separately: it lives inside the raw bytes, which are the only copy that is exactly what the
//! server sent. `ui.rs` is deliberately thin, and this does I/O, so it is neither there nor in
//! `view.rs` — which is I/O-free on purpose.

use mail_domain::Message;
use mail_store::SqliteStore;

/// The HTML part of a message, when it has one.
///
/// `None` on anything that goes wrong — a blob that is missing, bytes that do not parse, or a
/// message that is simply plain text. All three mean the same thing to the reader: show the
/// text part. A message the parser chokes on must still be readable, because the alternative is
/// a blank pane for mail that every other client displays fine.
///
/// Not cached, and not stored at ingest. Sanitized HTML must never be persisted — an `ammonia`
/// upgrade would otherwise leave every previously-ingested message sanitized under the old
/// rules — and persisting the *unsanitized* part would duplicate bytes the blob already holds.
pub fn html_of(store: &SqliteStore, message: &Message) -> Option<String> {
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(&store.connection(), raw).ok()?;
    mail_mime::parse(&bytes).ok()?.html
}
