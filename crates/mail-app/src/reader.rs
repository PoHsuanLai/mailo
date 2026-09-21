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

/// Everything the reader needs for one message, resolved.
///
/// Sanitize first, then resolve `cid:` inside the result. That order is the point: the
/// sanitizer must judge the URLs the *message* contains, not a `data:` URI substituted for one
/// — and running it second would mean re-judging bytes this code produced rather than bytes the
/// sender did.
pub fn render(store: &SqliteStore, message: &Message, policy: SanitizePolicy) -> Reading {
    // Parsed once. `html_of` and `inline_parts` each read the blob and run the whole MIME
    // parser, and `render` called both — so every message in an open thread was parsed twice on
    // every render, and the shell re-renders the open thread on every keystroke in the search
    // box. On a thread of large messages that is the difference between a reader that opens and
    // one that stutters.
    let Some(parsed) = parse_body(store, message) else {
        return reading(&message.body, None, policy);
    };
    match reading(&message.body, parsed.html.as_deref(), policy) {
        Reading::Html(sanitized) => Reading::Html(mail_mime::embed_inline(
            &sanitized,
            &parsed.attachments,
            mail_mime::INLINE_BUDGET,
        )),
        other => other,
    }
}

/// The message's stored bytes, parsed.
///
/// `None` for a body that has not arrived, a blob that is missing, or bytes that do not parse —
/// the reader's answer to all three is the same, and it is the text part.
fn parse_body(store: &SqliteStore, message: &Message) -> Option<mail_mime::Parsed> {
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(&store.connection(), raw).ok()?;
    mail_mime::parse(&bytes).ok()
}
