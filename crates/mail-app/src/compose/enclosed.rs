//! Forwarding a message as an attachment: the message itself, carried whole as a
//! `message/rfc822` part (RFC 2046 §5.2.1), rather than its text written out beneath a header
//! block.
//!
//! What goes is the stored raw message, byte for byte, so the recipient sees every header and
//! every part as they arrived here. That is only true of bytes that *are* the message as it was
//! sent, and two kinds of stored message are not: one whose body was never downloaded, and a
//! large IMAP message rebuilt from its parts with its attachments left on the server
//! (`mail_domain::Body::Present`'s `raw`). Both are refused, and the refusal says why and what
//! to do instead: attaching a rebuilt one would send a stand-in with empty attachments, under
//! the original's name.

use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::{ATTACHMENT_BUDGET, identity_of, save, signed};

/// How a forward carries the message it passes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carry {
    /// Its text, beneath a `Forwarded message` header block.
    Inline,
    /// The message itself, as a `message/rfc822` attachment.
    Attached,
}

/// The media type an attached message goes as.
pub const ENCLOSED: &str = "message/rfc822";

/// Whether `message`'s stored bytes `raw` were rebuilt from its parts rather than downloaded
/// whole, so they are not the message as it was sent.
///
/// Either says so: an attachment still on the server, or the markers the rebuild wrote into the
/// bytes, which stay after every such attachment has since been fetched.
pub fn rebuilt(message: &Message, raw: &[u8]) -> bool {
    message
        .attachments
        .iter()
        .any(|a| matches!(a.content, PartContent::Remote { .. }))
        || mail_mime::left_on_server(raw)
}

/// Create and persist a forward of `message` that carries it as an attachment, addressed to
/// `to`, with `body` as the covering note.
///
/// The draft is what [`super::draft_forward`] makes, subject and `forward_of` included, with
/// the note (signed) as its text and the message as its one attachment. Refused, with the reason
/// in words, when the stored bytes are not the message as it was sent.
pub fn draft_forward_attached(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let original = store.message(message).map_err(|e| e.to_string())?;
    let raw = sent_bytes(store, &original)?;
    let identity = identity_of(store, original.account, None)?;

    let mut draft = Draft::forward_of(&original, &identity, now);
    draft.to = to.to_vec();
    draft.text = signed(body, &identity);
    draft.attachments.push(PendingAttachment {
        name: file_name(&original.subject),
        mime: ENCLOSED.to_owned(),
        blob: raw,
    });
    save(store, &draft)?;
    Ok(draft)
}

/// The blob holding `message` as it was sent, or why there is none.
fn sent_bytes(store: &SqliteStore, message: &Message) -> Result<BlobId, String> {
    let Body::Present { raw, .. } = message.body else {
        return Err(
            "that message has not been downloaded yet, so there is nothing to attach. \
                    `mailo sync` downloads it; or forward it inline"
                .to_owned(),
        );
    };
    let bytes = store
        .blobs()
        .get(&store.connection(), raw)
        .map_err(|e| e.to_string())?;
    if rebuilt(message, &bytes) {
        return Err(
            "that message is large, so it was downloaded in parts and its attachments \
                    were left on the server. What is here is rebuilt from those parts, with the \
                    attachments empty, and attaching it would send that rather than the message \
                    as it was sent. Forward it inline instead"
                .to_owned(),
        );
    }
    let size = bytes.len() as u64;
    if size > ATTACHMENT_BUDGET {
        return Err(format!(
            "that message is {}, and most servers refuse above {}. Forward it inline instead",
            crate::attach::human_size(size),
            crate::attach::human_size(ATTACHMENT_BUDGET),
        ));
    }
    Ok(raw)
}

/// The attached message's file name: its subject, with `.eml`.
///
/// A slash would otherwise make [`crate::attach::safe_name`] keep only what follows it, so
/// "Q3/Q4 plan" would go as "Q4 plan.eml".
fn file_name(subject: &str) -> String {
    let subject: String = subject
        .chars()
        .map(|c| if matches!(c, '/' | '\\') { '-' } else { c })
        .collect();
    let subject = subject.trim();
    let stem = if subject.is_empty() {
        "message"
    } else {
        subject
    };
    crate::attach::safe_name(&format!("{stem}.eml"))
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn the_file_is_named_for_the_subject() {
        const CASES: &[(&str, &str)] = &[
            ("Lunch on Friday", "Lunch on Friday.eml"),
            ("Q3/Q4 plan", "Q3-Q4 plan.eml"),
            ("   ", "message.eml"),
            ("", "message.eml"),
            ("line\r\nbreak", "linebreak.eml"),
        ];
        for (subject, expect) in CASES {
            assert_eq!(file_name(subject), *expect, "{subject:?}");
        }
    }
}
