//! What the reader needs from the store that is not already a column.
//!
//! One function so far, and it exists because the HTML part of a message is not stored
//! separately: it lives inside the raw bytes, which are the only copy that is exactly what the
//! server sent. `ui.rs` is deliberately thin, and this does I/O, so it is neither there nor in
//! `view.rs` — which is I/O-free on purpose.

use crate::view::{Reading, reading};
use mail_domain::Message;
use mail_mime::SanitizePolicy;
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

/// The message's own inline parts, for resolving `cid:` references.
///
/// Read from the same raw bytes as the HTML, so a reference can only ever name a part of the
/// message that made it. Nothing here is keyed on anything the message says about the
/// filesystem, because nothing here touches the filesystem.
pub fn inline_parts(store: &SqliteStore, message: &Message) -> Vec<mail_mime::ParsedPart> {
    let Some(raw) = message.body.raw() else {
        return Vec::new();
    };
    let Ok(bytes) = store.blobs().get(&store.connection(), raw) else {
        return Vec::new();
    };
    match mail_mime::parse(&bytes) {
        Ok(parsed) => parsed.attachments,
        Err(_) => Vec::new(),
    }
}

/// Everything the reader needs for one message, resolved.
///
/// Sanitize first, then resolve `cid:` inside the result. That order is the point: the
/// sanitizer must judge the URLs the *message* contains, not a `data:` URI substituted for one
/// — and running it second would mean re-judging bytes this code produced rather than bytes the
/// sender did.
pub fn render(store: &SqliteStore, message: &Message, policy: SanitizePolicy) -> Reading {
    let html = html_of(store, message);
    match reading(&message.body, html.as_deref(), policy) {
        Reading::Html(sanitized) => Reading::Html(mail_mime::embed_inline(
            &sanitized,
            &inline_parts(store, message),
            mail_mime::INLINE_BUDGET,
        )),
        other => other,
    }
}
