//! Domain values to RFC 5322 bytes.

use crate::MimeError;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MailAddress;
use mail_builder::headers::content_type::ContentType;
use mail_builder::headers::raw::Raw;
use mail_builder::mime::{BodyPart, MimePart};
use mail_domain::{
    Address, BlobId, DeliveryReport, Draft, DraftId, Identity, Message, PendingAttachment,
    ReceiptRequest, normalize_id,
};
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
    /// Whether the envelope asks for delivery reports. Beside the envelope it belongs to, not in
    /// the bytes: RFC 3461's request is made of `RCPT TO`, and no header can make it.
    pub delivery: DeliveryReport,
}

/// The octets a part of `octets` becomes in base64 as [`build`] writes it: four for every three,
/// in lines of 76 each ended by CRLF (RFC 2045 §6.8). What an attachment costs on the wire.
pub fn base64_len(octets: u64) -> u64 {
    octets.div_ceil(3) * 4 + octets.div_ceil(57) * 2
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
    let rcpt_to = envelope(draft.to.iter().chain(&draft.cc).chain(&draft.bcc))?;
    Ok(Posting {
        mail_from: identity.from.email.trim().to_owned(),
        rcpt_to,
        message: build(draft, identity, in_reply_to, parts, Disclosure::HideBlind)?,
        delivery: draft.delivery,
    })
}

/// `RCPT TO` for `addresses`: each mailbox once, the first spelling kept, blanks dropped. Never
/// empty: a message to nobody is [`MimeError::NoRecipients`].
fn envelope<'a>(addresses: impl Iterator<Item = &'a Address>) -> Result<Vec<String>, MimeError> {
    let mut rcpt_to: Vec<String> = Vec::new();
    for addr in addresses {
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
    Ok(rcpt_to)
}

/// Whether a field of the original stays behind when it is redirected ([`resent`]).
fn left_behind(field: &[u8]) -> bool {
    let Some(colon) = field.iter().position(|&b| b == b':') else {
        return false;
    };
    let name = field[..colon].trim_ascii_end();
    name.eq_ignore_ascii_case(b"Bcc") || name.eq_ignore_ascii_case(b"Return-Path")
}

/// Who a message is redirected by and to, when, and the id the redirect goes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resending<'a> {
    /// The person redirecting it: `Resent-From`, and the envelope's `MAIL FROM`.
    pub from: &'a Address,
    /// Whom it goes to now: `Resent-To`, and the whole of `RCPT TO`.
    pub to: &'a [Address],
    /// `Resent-Date`. Set again as the message leaves, as a composed one's `Date` is
    /// ([`crate::restamp`]).
    pub at: chrono::DateTime<chrono::Utc>,
    /// What `Resent-Message-ID` is made from, with the sender's domain, as a composed
    /// message's `Message-ID` is made from its draft's: the outbox's entry goes by it too.
    pub id: DraftId,
}

/// `original` redirected (RFC 5322 §3.6.6): every byte of it as it came, beneath a `Resent-`
/// block that says who passed it on, to whom, when, and under what id.
///
/// Not a forward. Nothing of the original is rewritten, quoted or wrapped: its `From`, `To`,
/// `Date`, `Message-ID` and body are its author's, so a reply from the new recipient goes to the
/// author and threads under the original. The block goes first because the standard prepends
/// each redirect's block, newest on top, and because that is how [`crate::restamp`] knows the
/// message for one. The envelope is the new recipients alone: the original's `To` and `Cc` were
/// delivered to once already.
///
/// Two fields are the exception, and both are about what the original's journey left on it
/// rather than what its author wrote. `Bcc`, which only the sender's own copy carries: redirecting
/// a message from Sent would otherwise tell the new recipients whom its author blind-copied.
/// And `Return-Path`, which the last server put on at delivery and which RFC 5321 §4.4 says a
/// message being submitted must not already carry: the next server writes its own.
pub fn resent(original: &[u8], resending: &Resending<'_>) -> Result<Posting, MimeError> {
    use mail_builder::headers::Header;
    use mail_builder::headers::date::Date;
    use mail_builder::headers::message_id::MessageId;
    let rcpt_to = envelope(resending.to.iter())?;
    let mut block: Vec<u8> = Vec::with_capacity(256);
    let mut field = |name: &str, value: &dyn Fn(&mut Vec<u8>, usize)| {
        block.extend_from_slice(name.as_bytes());
        block.extend_from_slice(b": ");
        value(&mut block, name.len() + 2);
    };
    field("Resent-Date", &|out, column| {
        Date::new(resending.at.timestamp()).write_header(out, column)
    });
    field("Resent-From", &|out, column| {
        mail_addr(resending.from).write_header(out, column)
    });
    field("Resent-To", &|out, column| {
        mail_list(resending.to).write_header(out, column)
    });
    let id = id_at(resending.id, resending.from);
    field("Resent-Message-ID", &|out, column| {
        MessageId::new(id.as_str()).write_header(out, column)
    });
    let mut message = block;
    let (head, body) = crate::stamp::split_head(original);
    for kept in crate::stamp::fields(head)
        .into_iter()
        .filter(|field| !left_behind(field))
    {
        message.extend_from_slice(kept);
    }
    message.extend_from_slice(body);
    Ok(Posting {
        mail_from: mailbox(&resending.from.email),
        rcpt_to,
        message,
        // A redirect asks for no delivery report: it is the original passed on, not a message
        // of the sender's own.
        delivery: DeliveryReport::Unrequested,
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
    write(draft, identity, in_reply_to, parts, disclosure)
}

/// The bytes of `draft` as a copy for the server's Drafts mailbox: every addressee named, `Bcc`
/// included, since only the sender reads it there ([`Disclosure::Full`]).
///
/// No recipient is needed, unlike [`build`]: a draft is often kept before anyone is addressed,
/// and that is the copy another device finishes. Its `Message-ID` is [`draft_message_id`], the
/// same in every copy, which is what finds the earlier ones to replace.
pub fn draft_copy(
    draft: &Draft,
    identity: &Identity,
    in_reply_to: Option<&Message>,
    parts: &[(BlobId, Vec<u8>)],
) -> Result<Vec<u8>, MimeError> {
    write(draft, identity, in_reply_to, parts, Disclosure::Full)
}

/// The `Message-ID` every message built from `draft` carries, without angle brackets: the same
/// however often the draft is rebuilt, and the same when it is sent.
pub fn draft_message_id(draft: &Draft, identity: &Identity) -> String {
    message_id(draft, identity)
}

/// [`build`] without the check for a recipient.
fn write(
    draft: &Draft,
    identity: &Identity,
    in_reply_to: Option<&Message>,
    parts: &[(BlobId, Vec<u8>)],
    disclosure: Disclosure,
) -> Result<Vec<u8>, MimeError> {
    let html = draft.html.as_deref();
    let mut resolved = Vec::with_capacity(draft.attachments.len());
    // An image the body no longer shows (deleted, or never put back after an undo) stays on the
    // draft so an undo can bring it back, and is not sent: a part nothing refers to would arrive
    // as a stray file.
    for attachment in draft
        .attachments
        .iter()
        .filter(|attachment| placement(attachment, html) != Placement::Dropped)
    {
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
        .message_id(message_id(draft, identity));
    // `Draft.text` is always `format=flowed` (RFC 3676): the editor writes it so, and text
    // written any other way is made so before it is kept. Unlabelled, every soft break the
    // editor made at 72 columns would read as a hard one.
    builder.text_body = Some(MimePart::new(
        ContentType::new("text/plain")
            .attribute("charset", "utf-8")
            .attribute("format", "flowed")
            .attribute("delsp", "no"),
        BodyPart::Text(draft.text.as_str().into()),
    ));

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
    // Both spellings of urgency, and neither for Normal (`Priority::headers`). Written raw: the
    // values are this crate's own constants, never text from the draft.
    if let Some((x_priority, importance)) = draft.priority.headers() {
        builder = builder
            .header("X-Priority", Raw::new(x_priority))
            .header("Importance", Raw::new(importance));
    }
    // What a rule sent says so (RFC 3834), so no other machine's rule answers it in turn.
    if draft.automatic {
        builder = builder.header("Auto-Submitted", Raw::new("auto-replied"));
    }
    // No HTML part is a plain-text message: `text/plain` alone, not `multipart/alternative`.
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
    let mut inline = Vec::new();
    for (attachment, bytes) in resolved {
        let mime = media_type(&attachment.mime);
        if let Placement::Inline(cid) = placement(attachment, html) {
            // Inline, and named, so a reader that offers to save the picture has a name for it.
            let disposition =
                ContentType::new("inline").attribute("filename", filename(&attachment.name));
            inline.push(
                MimePart::new(mime, BodyPart::Binary(bytes.into()))
                    .header("Content-Disposition", disposition)
                    .cid(cid),
            );
            continue;
        }
        let enclosed = is_enclosed_message(mime);
        if enclosed && enclosed_encoding(&crlf_lines(bytes)) != "binary" {
            builder
                .attachments
                .get_or_insert_with(Vec::new)
                .push(enclosed_message(mime, filename(&attachment.name), bytes));
        } else {
            // A message SMTP cannot carry as itself goes as a file. `binary` is not an
            // encoding submission can negotiate here (RFC 3030 BINARYMIME), and base64 under
            // `message/rfc822` is forbidden (RFC 2046 §5.2.1), so the honest label is the
            // opaque one; the reader still gets every byte, to save and open.
            let mime = if enclosed {
                "application/octet-stream"
            } else {
                mime
            };
            builder = builder.attachment(mime, filename(&attachment.name), bytes);
        }
    }

    // RFC 2387: the HTML and the images it names travel together as one `multipart/related`,
    // the HTML first, in the place mail-builder gives the HTML body: so the message is
    // `alternative(text, related(html, images))`, inside `mixed` when files come with it.
    if !inline.is_empty()
        && let Some(html) = builder.html_body.take()
    {
        let mut parts = Vec::with_capacity(inline.len() + 1);
        parts.push(html);
        parts.extend(inline);
        // `type` names the root's type, which RFC 2387 §3.1 requires; the root is the HTML.
        let related = ContentType::new("multipart/related").attribute("type", "text/html");
        builder.html_body = Some(MimePart::new(related, parts));
    }

    builder
        .write_to_vec()
        .map_err(|err| MimeError::Unparseable(err.to_string()))
}

/// Where one of a draft's attachments goes in the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement<'a> {
    /// A file, after the body.
    Attached,
    /// An image the HTML shows, inside its `multipart/related`, under this `Content-ID`.
    Inline(&'a str),
    /// An image the HTML once showed and no longer does: not sent.
    Dropped,
}

/// Where `attachment` goes, given the HTML body. An image with a `Content-ID` is inline when the
/// HTML names it, and dropped when it does not; with no HTML to name it, or a `Content-ID` that
/// cannot be written into a header as it is, it goes as a file, so its bytes still arrive.
fn placement<'a>(attachment: &'a PendingAttachment, html: Option<&str>) -> Placement<'a> {
    let (Some(cid), Some(html)) = (attachment.cid.as_deref(), html) else {
        return Placement::Attached;
    };
    if !is_content_id(cid) {
        Placement::Attached
    } else if names_cid(html, cid) {
        Placement::Inline(cid)
    } else {
        Placement::Dropped
    }
}

/// Whether `cid` can be a `Content-ID` written raw between angle brackets: printable ASCII with
/// no space, bracket or quote, so it neither breaks the header nor the `src` that names it.
fn is_content_id(cid: &str) -> bool {
    !cid.is_empty()
        && cid
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'<' | b'>' | b'"' | b'\''))
}

/// Whether `html` refers to `cid:{cid}` as a whole reference, not as the start of a longer one.
fn names_cid(html: &str, cid: &str) -> bool {
    let wanted = format!("cid:{cid}");
    html.match_indices(&wanted).any(|(at, _)| {
        html[at + wanted.len()..]
            .chars()
            .next()
            .is_none_or(|next| matches!(next, '"' | '\'' | '>' | ')') || next.is_whitespace())
    })
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
    id_at(draft.id, &identity.from)
}

/// `{id}@{domain}`, the domain `from`'s when it is safe to put in a header unquoted.
fn id_at(id: DraftId, from: &Address) -> String {
    let domain = from
        .email
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| is_domain(domain))
        .unwrap_or("localhost");
    format!("{id}@{domain}").to_ascii_lowercase()
}

pub fn is_domain(domain: &str) -> bool {
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
/// server cannot carry it). A NUL or a line over 998 octets needs `binary`, which SMTP cannot
/// carry, so such a message is not enclosed this way at all (see [`build`]).
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
