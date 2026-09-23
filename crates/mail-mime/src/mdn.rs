//! Read receipts: whether a message asks for one, and the report that answers it (RFC 8098).
//!
//! Both halves read the original's raw bytes, never a stored [`mail_domain::Message`]: the
//! headers that matter here (`Disposition-Notification-To`, `Return-Path`, `Original-Recipient`)
//! are not fields of the domain message, and adding three columns to every stored message to
//! answer a question asked of a handful is the wrong trade.
//!
//! Nothing here decides to send. [`receipt`] builds bytes when asked; the asking is the user's.

use crate::MimeError;
use crate::build::Posting;
use chrono::{DateTime, Utc};
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MailAddress;
use mail_builder::headers::raw::Raw;
use mail_builder::mime::{BodyPart, MimePart};
use mail_domain::{Address, DraftId, Identity, normalize_id};
use mail_parser::{HeaderName, HeaderValue, Message, MessageParser, MimeHeaders};
use std::fmt::Write as _;

/// A message's request for a read receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptAsk {
    /// Where the receipt would go: the `Disposition-Notification-To` addresses. Never empty.
    pub to: Vec<Address>,
    /// How that address compares with the message's return path.
    pub return_path: ReturnPath,
}

/// Whether the receipt would go where the message came from.
///
/// RFC 8098 §2.1: a request whose address differs from the `Return-Path` is one to be wary of,
/// since anyone can write the header and a receipt confirms to its reader that this mailbox
/// exists and is read. Compared by domain: a list or a newsletter routinely has a `bounce+…`
/// return path on its own domain, and that is not what the warning is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnPath {
    /// Every receipt address is on the return path's domain.
    Agrees,
    /// At least one is not. Show this before asking.
    Differs { return_path: String },
    /// No usable `Return-Path` (absent, or the null `<>`): nothing to compare with.
    Unknown,
}

/// Whether to carry the original's headers in the receipt's third part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalHeaders {
    /// A `text/rfc822-headers` part with the fields the original's *author* wrote — `From`,
    /// `To`, `Cc`, `Subject`, `Date`, `Message-ID`, `In-Reply-To`, `References`. Not the
    /// `Received` trail or anything a server added: those describe the reader's own mail
    /// system, and a receipt is not the place to hand that to a stranger.
    Included,
    /// Two parts only.
    Omitted,
}

/// What [`receipt`] needs besides the original.
#[derive(Debug, Clone, Copy)]
pub struct Reporting<'a> {
    /// Who displayed the message: `From` of the receipt, and its `Final-Recipient`.
    pub reader: &'a Identity,
    /// `Reporting-UA`, e.g. `"mailo; mailo 0.1.0"`.
    pub agent: &'a str,
    /// Names the receipt. Its `Message-ID` and MIME boundary are derived from it, so the same
    /// inputs build the same bytes.
    pub id: DraftId,
    /// When the receipt was written.
    pub at: DateTime<Utc>,
    pub headers: OriginalHeaders,
}

/// Whether `raw` asks for a read receipt, and to where.
///
/// `None` when it does not ask, when every address it names is unusable, or when the message
/// is itself a disposition notification — RFC 8098 §3 forbids answering one, which is what
/// stops two clients receipting each other's receipts for ever.
pub fn receipt_asked(raw: &[u8]) -> Option<ReceiptAsk> {
    let raw = crate::charset::headers_as_utf8(raw).unwrap_or_else(|| raw.to_vec());
    let message = headers_parser().parse_headers(&raw)?;
    if is_disposition_report(&message) {
        return None;
    }
    let to = addresses(&message, HeaderName::DispositionNotificationTo);
    if to.is_empty() {
        return None;
    }
    let return_path = compare(&to, return_path(&message));
    Some(ReceiptAsk { to, return_path })
}

/// The receipt for `original`, ready to queue: RFC 8098 `multipart/report;
/// report-type=disposition-notification`, disposition `manual-action/MDN-sent-manually;
/// displayed`.
///
/// Addressed to the original's `Disposition-Notification-To`; [`MimeError::MissingHeader`]
/// when it has none, or is itself a receipt.
///
/// `MAIL FROM` is the reader's own address, where RFC 8098 §3 asks for the null path `<>`.
/// This goes through the account's submission server like any other message, and submission
/// servers bind the envelope sender to the signed-in user (RFC 6409 §6.1) and refuse or rewrite
/// a null one. What the null path prevents — a bounce of the receipt — reaches only the reader.
pub fn receipt(original: &[u8], reporting: &Reporting<'_>) -> Result<Posting, MimeError> {
    let ask =
        receipt_asked(original).ok_or(MimeError::MissingHeader("Disposition-Notification-To"))?;
    let decoded = crate::charset::headers_as_utf8(original);
    let original = decoded.as_deref().unwrap_or(original);
    let message = headers_parser()
        .parse_headers(original)
        .ok_or_else(|| MimeError::Unparseable("no RFC 5322 header was found".to_owned()))?;

    let reader = &reporting.reader.from;
    let original_id = first_id(&message, HeaderName::MessageId);
    let subject = message.subject().unwrap_or("").trim();
    let domain = reader
        .email
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| crate::build::is_domain(domain))
        .unwrap_or("localhost");
    let id = format!("{}@{domain}", reporting.id).to_ascii_lowercase();

    let mut parts = vec![
        MimePart::new("text/plain", human_text(reader, subject, &message)),
        MimePart::new(
            "message/disposition-notification",
            BodyPart::Binary(
                notification(reporting, &message, original_id.as_deref())
                    .into_bytes()
                    .into(),
            ),
        )
        .transfer_encoding("7bit"),
    ];
    if reporting.headers == OriginalHeaders::Included {
        parts.push(original_headers(original, &message));
    }

    let rcpt_to: Vec<String> = ask
        .to
        .iter()
        .map(|addr| crate::build::mailbox(&addr.email))
        .filter(|email| !email.is_empty())
        .collect();
    if rcpt_to.is_empty() {
        return Err(MimeError::NoRecipients);
    }

    let mut builder = MessageBuilder::new()
        .from(crate::build::mail_addr(reader))
        .to(MailAddress::new_list(
            ask.to.iter().map(crate::build::mail_addr).collect(),
        ))
        .subject(format!("Read: {subject}"))
        .date(reporting.at.timestamp())
        .message_id(id)
        // RFC 3834: generated in answer to a message rather than written as one. Keeps a
        // vacation responder from answering the receipt.
        .header("Auto-Submitted", Raw::new("auto-replied"));
    if let Some(parent) = &original_id {
        builder = builder
            .in_reply_to(parent.clone())
            .references(parent.clone());
    }
    let report = mail_builder::headers::content_type::ContentType::new("multipart/report")
        .attribute("report-type", "disposition-notification")
        .attribute("boundary", format!("mdn-{}", reporting.id));
    let bytes = builder
        .body(MimePart::new(report, parts))
        .write_to_vec()
        .map_err(|err| MimeError::Unparseable(err.to_string()))?;

    Ok(Posting {
        mail_from: crate::build::mailbox(&reader.email),
        rcpt_to,
        message: bytes,
    })
}

/// The part a person reads. Says what a receipt means and, as plainly, what it does not.
///
/// Broken into short lines by hand, so an ordinary receipt stays 7-bit text rather than
/// quoted-printable that a person reading the source has to decode.
fn human_text(reader: &Address, subject: &str, original: &Message<'_>) -> String {
    let mut out = format!(
        "This is a receipt for the message you sent to {}",
        reader.email
    );
    if let Some(date) = original.date().filter(|date| date.is_valid()) {
        let _ = write!(out, "\r\non {}", date.to_rfc822());
    }
    if subject.is_empty() {
        out.push_str(" with no subject.\r\n");
    } else {
        let _ = write!(out, " with the subject\r\n\"{subject}\".\r\n");
    }
    out.push_str(
        "\r\nIt was displayed on the recipient's screen. That says nothing\r\n\
         about whether it was read, understood or agreed with.\r\n",
    );
    out
}

/// The `message/disposition-notification` fields, RFC 8098 §3.1.
fn notification(reporting: &Reporting<'_>, original: &Message<'_>, id: Option<&str>) -> String {
    let mut out = String::new();
    let _ = write!(out, "Reporting-UA: {}\r\n", one_line(reporting.agent));
    // Copied when the original carried one (§3.2.3): it is what the sender's own system called
    // this recipient, which can differ from the address the message finally reached.
    if let Some(recipient) = raw_field(original, "Original-Recipient") {
        let _ = write!(out, "Original-Recipient: {}\r\n", one_line(&recipient));
    }
    let _ = write!(
        out,
        "Final-Recipient: rfc822;{}\r\n",
        crate::build::mailbox(&reporting.reader.from.email)
    );
    if let Some(id) = id {
        let _ = write!(out, "Original-Message-ID: <{id}>\r\n");
    }
    out.push_str("Disposition: manual-action/MDN-sent-manually; displayed\r\n");
    out
}

/// The originator fields the original's author wrote, as the `text/rfc822-headers` part.
fn original_headers(original: &[u8], message: &Message<'_>) -> MimePart<'static> {
    const KEPT: &[&str] = &[
        "from",
        "to",
        "cc",
        "subject",
        "date",
        "message-id",
        "in-reply-to",
        "references",
    ];
    let mut block = Vec::new();
    for header in message.headers() {
        if !KEPT
            .iter()
            .any(|name| header.name.as_str().eq_ignore_ascii_case(name))
        {
            continue;
        }
        // The field exactly as it arrived, name through line ending.
        let start = header.offset_field as usize;
        let end = header.offset_end as usize;
        if let Some(field) = original.get(start..end) {
            block.extend_from_slice(field);
            if !field.ends_with(b"\n") {
                block.extend_from_slice(b"\r\n");
            }
        }
    }
    let part = MimePart::new(
        "text/rfc822-headers",
        BodyPart::Binary(block.clone().into()),
    );
    if block.is_ascii() {
        part.transfer_encoding("7bit")
    } else {
        // Base64 rather than raw 8-bit: the receipt then needs no 8BITMIME from anyone.
        part
    }
}

fn headers_parser() -> MessageParser {
    MessageParser::new()
        .with_mime_headers()
        .with_message_ids()
        .header_date(HeaderName::Date)
        .header_text(HeaderName::Subject)
        .header_address(HeaderName::From)
        .header_address(HeaderName::DispositionNotificationTo)
        .header_address(HeaderName::ReturnPath)
}

fn is_disposition_report(message: &Message<'_>) -> bool {
    message.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case("multipart")
            && ct
                .subtype()
                .is_some_and(|sub| sub.eq_ignore_ascii_case("report"))
            && ct
                .attribute("report-type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("disposition-notification"))
    })
}

fn addresses(message: &Message<'_>, name: HeaderName<'static>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new();
    for value in message.header_values(name) {
        let Some(list) = value.as_address() else {
            continue;
        };
        for addr in list.iter() {
            let Some(email) = addr.address().map(str::trim).filter(|e| is_mailbox(e)) else {
                continue;
            };
            if out
                .iter()
                .any(|kept| kept.email.eq_ignore_ascii_case(email))
            {
                continue;
            }
            out.push(Address {
                name: addr
                    .name()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                email: email.to_owned(),
            });
        }
    }
    out
}

/// The topmost `Return-Path` — the one the final delivery wrote — when it names a mailbox.
fn return_path(message: &Message<'_>) -> Option<String> {
    let value = message.header_values(HeaderName::ReturnPath).next()?;
    let addr = value.as_address()?.first()?.address()?.trim();
    is_mailbox(addr).then(|| addr.to_owned())
}

fn compare(to: &[Address], return_path: Option<String>) -> ReturnPath {
    let Some(path) = return_path else {
        return ReturnPath::Unknown;
    };
    let path_domain = domain_of(&path);
    if to
        .iter()
        .all(|addr| domain_of(&addr.email).eq_ignore_ascii_case(path_domain))
    {
        ReturnPath::Agrees
    } else {
        ReturnPath::Differs { return_path: path }
    }
}

fn domain_of(email: &str) -> &str {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim_end_matches('.'))
        .unwrap_or("")
}

/// Something with a local part, an `@` and a domain, and no control characters.
fn is_mailbox(email: &str) -> bool {
    email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !email.chars().any(char::is_control)
    })
}

fn first_id(message: &Message<'_>, name: HeaderName<'static>) -> Option<String> {
    message
        .header_values(name)
        .filter_map(HeaderValue::as_text_list)
        .flatten()
        .map(|raw| normalize_id(raw))
        .find(|id| !id.is_empty())
}

/// An unstructured field's value by name, trimmed, when present and not empty.
fn raw_field(message: &Message<'_>, name: &str) -> Option<String> {
    message
        .headers()
        .iter()
        .find(|h| h.name.as_str().eq_ignore_ascii_case(name))
        .and_then(|h| h.value.as_text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// A value safe to write after a field name: cut at the first control character, so a hostile
/// value cannot start a header of its own.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
