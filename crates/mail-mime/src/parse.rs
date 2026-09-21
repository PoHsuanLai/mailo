//! RFC 5322 / MIME bytes to domain values.

use crate::MimeError;
use chrono::{DateTime, Utc};
use mail_domain::{Address, Inline, normalize_id};
use mail_parser::{HeaderName, Message, MessageParser, MimeHeaders, PartType};
use std::collections::HashSet;

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
pub fn parse(raw: &[u8]) -> Result<Parsed, MimeError> {
    // A header is `name ":" value`. Bytes with no colon are not a message; the parser
    // will otherwise treat a run of NULs as a header name and hand back an empty shell.
    if !raw.contains(&b':') {
        return Err(not_a_message());
    }
    // `MessageParser::default()` has an empty header map, which selects the built-in
    // structured parsers (dates, addresses, ids, MIME). It returns `None` only when it
    // could not find a single header.
    let Some(message) = MessageParser::default().parse(raw) else {
        return Err(not_a_message());
    };
    if message.parts.is_empty() {
        return Err(not_a_message());
    }
    Ok(assemble(&message))
}

fn not_a_message() -> MimeError {
    MimeError::Unparseable("no RFC 5322 header was found".to_owned())
}

fn assemble(message: &Message<'_>) -> Parsed {
    let (text, html, attachments) = bodies(message);
    Parsed {
        rfc_message_id: first_id(message, HeaderName::MessageId),
        date: message_date(message),
        from: addresses(message, HeaderName::From).into_iter().next(),
        reply_to: addresses(message, HeaderName::ReplyTo),
        to: addresses(message, HeaderName::To),
        cc: addresses(message, HeaderName::Cc),
        bcc: addresses(message, HeaderName::Bcc),
        subject: message.subject().unwrap_or("").to_owned(),
        // One slot. The last id is the immediate parent; threading appends this one value
        // after `references`, so an earlier id would point the reply at the wrong message.
        in_reply_to: last_id(message, HeaderName::InReplyTo),
        references: reference_ids(message),
        text,
        html,
        attachments,
    }
}

fn message_date(message: &Message<'_>) -> Option<DateTime<Utc>> {
    let parsed = message.date()?;
    if !parsed.is_valid() {
        return None;
    }
    DateTime::from_timestamp(parsed.to_timestamp(), 0)
}

fn addresses(message: &Message<'_>, name: HeaderName<'static>) -> Vec<Address> {
    let mut out = Vec::new();
    for value in message.header_values(name) {
        let Some(list) = value.as_address() else {
            continue;
        };
        for addr in list.iter() {
            let Some(email) = addr
                .address()
                .map(str::trim)
                .filter(|email| !email.is_empty())
            else {
                continue;
            };
            let name = addr
                .name()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
            out.push(Address {
                name,
                email: email.to_owned(),
            });
        }
    }
    out
}

/// Oldest first. An unusable id (empty after [`normalize_id`]) is absent, not stored.
/// The first spelling of a repeated id wins, which keeps the oldest position.
fn reference_ids(message: &Message<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in message.header_values(HeaderName::References) {
        let Some(list) = value.as_text_list() else {
            continue;
        };
        for raw in list {
            let id = normalize_id(raw);
            if id.is_empty() || !seen.insert(id.clone()) {
                continue;
            }
            out.push(id);
        }
    }
    out
}

fn first_id(message: &Message<'_>, name: HeaderName<'static>) -> Option<String> {
    for value in message.header_values(name) {
        let Some(list) = value.as_text_list() else {
            continue;
        };
        for raw in list {
            let id = normalize_id(raw);
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

fn last_id(message: &Message<'_>, name: HeaderName<'static>) -> Option<String> {
    let mut last = None;
    for value in message.header_values(name) {
        let Some(list) = value.as_text_list() else {
            continue;
        };
        for raw in list {
            let id = normalize_id(raw);
            if !id.is_empty() {
                last = Some(id);
            }
        }
    }
    last
}

/// Text and HTML come from the parts mail-parser chose as bodies. A `text/plain` or
/// `text/html` part it filed as an attachment is lifted back when we have no body of that
/// kind and the part is not `Content-Disposition: attachment` — `multipart/related` does
/// this to a root part that is not first. An attached `.html` file stays an attachment.
fn bodies(message: &Message<'_>) -> (Option<String>, Option<String>, Vec<ParsedPart>) {
    let mut text = part_text(message.text_bodies());
    let mut html = part_html(message.html_bodies());
    let mut attachments = Vec::new();
    for part in message.attachments() {
        if matches!(part.body, PartType::Multipart(_)) {
            continue;
        }
        if text.is_none() && is_loose_text(part) {
            text = part.text_contents().map(str::to_owned);
            continue;
        }
        if html.is_none() && is_loose_html(part) {
            html = part.text_contents().map(str::to_owned);
            continue;
        }
        let index = attachments.len() + 1;
        attachments.push(ParsedPart {
            name: part_name(part, index),
            mime: part_mime(part),
            bytes: part.contents().to_vec(),
            inline: part_inline(part),
        });
    }
    (text, html, attachments)
}

fn part_text<'a>(
    mut parts: impl Iterator<Item = &'a mail_parser::MessagePart<'a>>,
) -> Option<String> {
    parts.find_map(|part| match &part.body {
        PartType::Text(text) => Some(text.to_string()),
        _ => None,
    })
}

fn part_html<'a>(
    mut parts: impl Iterator<Item = &'a mail_parser::MessagePart<'a>>,
) -> Option<String> {
    parts.find_map(|part| match &part.body {
        PartType::Html(html) => Some(html.to_string()),
        _ => None,
    })
}

fn is_loose_text(part: &mail_parser::MessagePart<'_>) -> bool {
    matches!(part.body, PartType::Text(_)) && part.content_id().is_none() && !is_attachment(part)
}

fn is_loose_html(part: &mail_parser::MessagePart<'_>) -> bool {
    matches!(part.body, PartType::Html(_)) && !is_attachment(part)
}

fn is_attachment(part: &mail_parser::MessagePart<'_>) -> bool {
    part.content_disposition()
        .is_some_and(|disposition| disposition.is_attachment())
}

fn part_name(part: &mail_parser::MessagePart<'_>, index: usize) -> String {
    part.attachment_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("part-{index}"))
}

fn part_mime(part: &mail_parser::MessagePart<'_>) -> String {
    let Some(ct) = part.content_type() else {
        return "application/octet-stream".to_owned();
    };
    let top = ct.ctype().trim();
    if top.is_empty() {
        return "application/octet-stream".to_owned();
    }
    match ct.subtype().map(str::trim).filter(|sub| !sub.is_empty()) {
        Some(sub) => format!("{top}/{sub}").to_ascii_lowercase(),
        None => top.to_ascii_lowercase(),
    }
}

/// `Content-ID` with the angle brackets removed. Case is preserved: a renderer matches
/// this string against `cid:` URLs, and message-ids' lowercasing rule does not apply.
fn part_inline(part: &mail_parser::MessagePart<'_>) -> Inline {
    let Some(raw) = part.content_id() else {
        return Inline::Attached;
    };
    let trimmed = raw.trim();
    let core = match (
        trimmed.starts_with('<'),
        trimmed.ends_with('>'),
        trimmed.len(),
    ) {
        (true, true, len) if len >= 2 => trimmed[1..len - 1].trim(),
        _ => trimmed,
    };
    if core.is_empty() {
        Inline::Attached
    } else {
        Inline::Embedded {
            cid: core.to_owned(),
        }
    }
}
