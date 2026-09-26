//! Domain values to RFC 5322 bytes.

use crate::MimeError;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MailAddress;
use mail_builder::mime::{BodyPart, MimePart};
use mail_domain::{Address, BlobId, Draft, Identity, Message, ReceiptRequest, normalize_id};
use std::collections::HashSet;

/// Whether the built bytes may name blind recipients.
///
/// Two callers want genuinely different bytes from one draft, and the difference is not a
/// detail: it is who learns that a blind copy was sent. Making it an argument means neither
/// caller can get it by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    /// Write every addressee into the headers, `Bcc` included.
    ///
    /// For a copy only the sender will read — the `Drafts` folder, or the `Sent` copy, where
    /// losing the record of who was blind-copied would be losing information the user wants.
    Full,
    /// Omit `Bcc` from the headers.
    ///
    /// For bytes going onto the wire. The blind recipients are carried by the envelope
    /// instead, so they still receive the message and nobody else learns they did.
    HideBlind,
}

/// Bytes for the wire, with the envelope that carries them.
///
/// The two are returned together on purpose. Deriving the envelope by re-parsing the message
/// is how a `Bcc` recipient silently stops receiving mail the moment the headers stop naming
/// them — the bug this type exists to make unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    /// `MAIL FROM`. The identity's own address, not any `Reply-To`: this is the return path
    /// for bounces, and pointing it at a list address sends the bounces to the list.
    pub mail_from: String,
    /// `RCPT TO`, one per envelope recipient: `To`, `Cc` **and** `Bcc`.
    pub rcpt_to: Vec<String>,
    /// The message, built with [`Disclosure::HideBlind`].
    pub message: Vec<u8>,
}

/// Everything needed to submit `draft`: the envelope and the bytes, consistent by construction.
///
/// Recipients are deduplicated case-insensitively on the domain, keeping the first spelling of
/// each mailbox. A duplicate `RCPT TO` is not an error at most servers, but it is a second
/// delivery to someone who was on both `To` and `Bcc`.
pub fn posting(
    draft: &Draft,
    identity: &Identity,
    in_reply_to: Option<&Message>,
    parts: &[(BlobId, Vec<u8>)],
) -> Result<Posting, MimeError> {
    let mut rcpt_to: Vec<String> = Vec::new();
    for addr in draft.to.iter().chain(&draft.cc).chain(&draft.bcc) {
        let email = addr.email.trim();
        if email.is_empty() {
            continue;
        }
        if !rcpt_to.iter().any(|kept| kept.eq_ignore_ascii_case(email)) {
            rcpt_to.push(email.to_owned());
        }
    }
    if rcpt_to.is_empty() {
        return Err(MimeError::NoRecipients);
    }
    Ok(Posting {
        mail_from: identity.from.email.trim().to_owned(),
        rcpt_to,
        message: build(draft, identity, in_reply_to, parts, Disclosure::HideBlind)?,
    })
}

/// Build the bytes to submit for `draft`.
///
/// `parts` supplies the contents of every [`mail_domain::PendingAttachment`] by `BlobId`;
/// a referenced blob that is absent is [`MimeError::MissingPart`] rather than a silent drop.
///
/// `in_reply_to` is the message being replied to, when there is one. It is needed for correct
/// `In-Reply-To` and `References` headers — threading is reconstructed by the *recipient's*
/// client from those, so getting them wrong breaks the conversation on their side, where we
/// will never see it.
pub fn build(
    draft: &Draft,
    identity: &Identity,
    in_reply_to: Option<&Message>,
    parts: &[(BlobId, Vec<u8>)],
    disclosure: Disclosure,
) -> Result<Vec<u8>, MimeError> {
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err(MimeError::NoRecipients);
    }

    let mut resolved = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let bytes = parts
            .iter()
            .find(|(id, _)| *id == attachment.blob)
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or_else(|| MimeError::MissingPart(attachment.blob.to_string()))?;
        resolved.push((attachment, bytes));
    }

    // `Date` is the draft's own timestamp. Leaving it unset makes mail-builder call
    // `SystemTime::now()`, and a library below `mail-runtime` does not read the clock.
    let mut builder = MessageBuilder::new()
        .from(mail_addr(&identity.from))
        .subject(draft.subject.as_str())
        .date(draft.updated.timestamp())
        .message_id(message_id(draft, identity))
        .text_body(draft.text.as_str());

    if let Some(reply_to) = &identity.reply_to {
        builder = builder.reply_to(mail_addr(reply_to));
    }
    if !draft.to.is_empty() {
        builder = builder.to(mail_list(&draft.to));
    }
    if !draft.cc.is_empty() {
        builder = builder.cc(mail_list(&draft.cc));
    }
    // The whole point of `Disclosure`. A `Bcc` header on bytes that go to the SMTP server is
    // handed to every ordinary recipient, which is the opposite of what the user asked for.
    // The blind addresses still receive the message: they are envelope recipients, which
    // `posting` puts in `rcpt_to` where the other recipients never see them.
    if !draft.bcc.is_empty() && disclosure == Disclosure::Full {
        builder = builder.bcc(mail_list(&draft.bcc));
    }
    // RFC 8098 §2.1. The receipt goes to the address the message is from, not to `Reply-To`:
    // it is the sender asking, and the recipient's client compares this header's domain with the
    // `Return-Path`, which is `from` too (see `Posting::mail_from`). A `Reply-To` on another
    // domain would make every such client warn that the request looks forged.
    if draft.receipt == ReceiptRequest::Requested {
        builder = builder.header("Disposition-Notification-To", mail_addr(&identity.from));
    }
    if let Some(html) = draft.html.as_deref() {
        builder = builder.html_body(html);
    }
    if let Some(parent) = in_reply_to {
        let (parent_id, references) = reply_headers(parent);
        if let Some(id) = parent_id {
            builder = builder.in_reply_to(id);
        }
        if !references.is_empty() {
            builder = builder.references(references);
        }
    }
    for (attachment, bytes) in resolved {
        let mime = media_type(&attachment.mime);
        if is_enclosed_message(mime) {
            builder
                .attachments
                .get_or_insert_with(Vec::new)
                .push(enclosed_message(mime, filename(&attachment.name), bytes));
        } else {
            builder = builder.attachment(mime, filename(&attachment.name), bytes);
        }
    }

    builder
        .write_to_vec()
        .map_err(|err| MimeError::Unparseable(err.to_string()))
}

/// `References` is the parent's `References` followed by the parent's `Message-ID`.
///
/// The parent id is moved to the end when it already appears. Parse keeps the first copy
/// of a duplicate, so leaving an earlier copy in place would make the recipient thread the
/// reply under that earlier message instead of under this parent.
fn reply_headers(parent: &Message) -> (Option<String>, Vec<String>) {
    let mut references = Vec::new();
    let mut seen = HashSet::new();
    for raw in &parent.references {
        let Some(id) = usable_id(raw) else {
            continue;
        };
        if seen.insert(id.clone()) {
            references.push(id);
        }
    }
    let parent_id = parent.rfc_message_id.as_deref().and_then(usable_id);
    if let Some(id) = &parent_id {
        references.retain(|existing| existing != id);
        references.push(id.clone());
    }
    (parent_id, references)
}

fn usable_id(raw: &str) -> Option<String> {
    let id = normalize_id(raw);
    if id.is_empty() { None } else { Some(id) }
}

/// Stable id from the draft, so the bytes do not depend on the clock. The domain half is
/// the sender's domain when that string is safe to put in a header unquoted.
fn message_id(draft: &Draft, identity: &Identity) -> String {
    let domain = identity
        .from
        .email
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| is_domain(domain))
        .unwrap_or("localhost");
    format!("{}@{domain}", draft.id).to_ascii_lowercase()
}

pub(crate) fn is_domain(domain: &str) -> bool {
    !domain.is_empty()
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && domain
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-'))
}

fn mail_list(addrs: &[Address]) -> MailAddress<'static> {
    MailAddress::new_list(addrs.iter().map(mail_addr).collect())
}

/// The mailbox is written raw inside `<>`. Bytes from the first control character on are
/// dropped, then angle brackets and whitespace are removed, so a value copied out of a
/// hostile header cannot start a new header line or glue a `Bcc` onto the address.
pub(crate) fn mail_addr(addr: &Address) -> MailAddress<'static> {
    let name = addr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    MailAddress::new_address(name, mailbox(&addr.email))
}

pub(crate) fn mailbox(email: &str) -> String {
    // Cut at the first control character. Deleting the breaks and keeping the tail
    // would glue `Bcc: victim` onto the real mailbox.
    email
        .chars()
        .take_while(|c| !c.is_ascii_control())
        .filter(|c| *c != '<' && *c != '>' && !c.is_whitespace())
        .collect()
}

fn filename(name: &str) -> String {
    name.chars()
        .take_while(|c| *c != '\r' && *c != '\n' && *c != '\0')
        .collect()
}

/// Whether `mime` is `message/rfc822`, parameters aside.
fn is_enclosed_message(mime: &str) -> bool {
    let head = mime.split_once(';').map_or(mime, |(head, _)| head).trim();
    head.eq_ignore_ascii_case("message/rfc822")
}

/// A whole message carried as a `message/rfc822` part.
///
/// Never base64 or quoted-printable, which is what mail-builder gives any part that is not
/// text: RFC 2046 §5.2.1 allows a `message/rfc822` body only `7bit`, `8bit` or `binary`, and a
/// reader that honours that shows an encoded one as an opaque file, not as a message. The bytes
/// go in as they are, with every line ending made CRLF (RFC 5322 §2.1: a message's lines end in
/// CRLF, and a bare LF from an mbox import would otherwise be a line break the wire does not
/// have). The label is the least the content needs: `7bit` for short ASCII lines, `8bit` when
/// there are 8-bit bytes (the submission then asks for `BODY=8BITMIME`, or is refused where the
/// server cannot carry it), and `binary` for a NUL or a line over 998 octets, which SMTP cannot
/// carry at all and which says so rather than claiming to be something it is not.
fn enclosed_message<'x>(mime: &'x str, name: String, bytes: &[u8]) -> MimePart<'x> {
    let body = crlf_lines(bytes);
    let encoding = enclosed_encoding(&body);
    MimePart::new(mime, BodyPart::Binary(body.into()))
        .attachment(name)
        .transfer_encoding(encoding)
}

/// `bytes` with every bare LF and bare CR made CRLF.
fn crlf_lines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 64);
    let mut at = 0;
    while at < bytes.len() {
        match (bytes[at], bytes.get(at + 1)) {
            (b'\r', Some(b'\n')) => {
                out.extend_from_slice(b"\r\n");
                at += 2;
            }
            (b'\r' | b'\n', _) => {
                out.extend_from_slice(b"\r\n");
                at += 1;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    out
}

/// The `Content-Transfer-Encoding` CRLF-lined `body` can honestly be sent under (RFC 2045 §2.7–2.9).
fn enclosed_encoding(body: &[u8]) -> &'static str {
    const MAX_LINE: usize = 998;
    let long = body
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_suffix(b"\r").unwrap_or(line).len() > MAX_LINE);
    if long || body.contains(&0) {
        "binary"
    } else if body.is_ascii() {
        "7bit"
    } else {
        "8bit"
    }
}

/// Parameters (`text/plain; charset=utf-8`) are kept. A type that is not `token/token`,
/// or any control character, is replaced: the content-type is written raw, so a newline
/// in the declared type would otherwise become a new header.
fn media_type(raw: &str) -> &str {
    let head = match raw.split_once(';') {
        Some((head, _)) => head.trim(),
        None => raw.trim(),
    };
    if is_media_type(head) && !raw.bytes().any(|byte| byte.is_ascii_control()) {
        raw
    } else {
        "application/octet-stream"
    }
}

fn is_media_type(raw: &str) -> bool {
    let Some((top, sub)) = raw.split_once('/') else {
        return false;
    };
    is_token(top) && is_token(sub)
}

fn is_token(token: &str) -> bool {
    !token.is_empty()
        && token.bytes().all(|byte| {
            matches!(
                byte,
                b'!' | b'#' | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'0'..=b'9'
                    | b'A'..=b'Z'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'a'..=b'z'
                    | b'{'
                    | b'|'
                    | b'}'
                    | b'~'
            )
        })
}
