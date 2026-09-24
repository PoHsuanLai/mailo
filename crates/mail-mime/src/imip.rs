//! Calendar invitations in mail (iMIP, RFC 6047): finding the calendar object a message carries,
//! and building the message that answers one.
//!
//! What the calendar object *says* is `mail-pim`'s business; this module only knows where it
//! sits in a message and how an answer is wrapped. Nothing here decides to answer.

use crate::MimeError;
use crate::build::Posting;
use chrono::{DateTime, Utc};
use mail_builder::MessageBuilder;
use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::{BodyPart, MimePart};
use mail_domain::{Address, DraftId, normalize_id};
use mail_parser::{HeaderName, HeaderValue, MessageParser, MimeHeaders, PartType};

/// The calendar object a message carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarPart {
    /// The part's `method` parameter (RFC 6047 §2.4), as written. The object's own `METHOD` is
    /// the one to believe; this is what a program that only read the MIME headers would see.
    pub method: Option<String>,
    /// The object, decoded to text.
    pub text: String,
}

/// The calendar object in `raw`, when it carries one.
///
/// The first `text/calendar` part wins, wherever it sits — an iMIP message puts it in a
/// `multipart/alternative` beside a text part a person reads. Failing one, the first
/// `application/ics` part, or attachment named `*.ics`, which is how some programs attach an
/// event rather than send one. A message forwarded inside this one is not looked into: an
/// invitation someone forwarded is theirs, not the reader's to answer.
pub fn calendar_part(raw: &[u8]) -> Option<CalendarPart> {
    let message = MessageParser::default().parse(raw)?;
    let mut fallback = None;
    for part in &message.parts {
        if matches!(part.body, PartType::Multipart(_) | PartType::Message(_)) {
            continue;
        }
        let (top, sub) = part
            .content_type()
            .map_or(("", ""), |ct| (ct.ctype(), ct.subtype().unwrap_or("")));
        let is = |t: &str, s: &str| top.eq_ignore_ascii_case(t) && sub.eq_ignore_ascii_case(s);
        let named_ics = part
            .attachment_name()
            .is_some_and(|name| name.trim().to_ascii_lowercase().ends_with(".ics"));
        if is("text", "calendar") {
            return Some(read_part(part));
        }
        if fallback.is_none() && (is("application", "ics") || named_ics) {
            fallback = Some(read_part(part));
        }
    }
    fallback
}

fn read_part(part: &mail_parser::MessagePart<'_>) -> CalendarPart {
    let method = part
        .content_type()
        .and_then(|ct| ct.attribute("method"))
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty());
    let text = match &part.body {
        // Decoded from its `charset` by the parser.
        PartType::Text(text) => text.to_string(),
        // An attachment: iCalendar's charset is UTF-8 (RFC 5545 §3.1.4).
        _ => {
            let bytes = part.contents();
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    CalendarPart { method, text }
}

/// What [`calendar_reply`] needs besides the invitation it answers.
#[derive(Debug, Clone, Copy)]
pub struct CalendarReply<'a> {
    /// The attendee answering: `From`, and the envelope sender.
    pub from: &'a Address,
    /// The organiser.
    pub to: &'a Address,
    /// "Accepted: Weekly sync", as calendars title an answer.
    pub subject: &'a str,
    /// What a person reading the answer in a mail program sees.
    pub text: &'a str,
    /// The `METHOD:REPLY` calendar object.
    pub calendar: &'a str,
    /// Names the message. Its `Message-ID` and MIME boundary are derived from it, so the same
    /// inputs build the same bytes.
    pub id: DraftId,
    pub at: DateTime<Utc>,
}

/// The message answering the invitation in `original`: a `multipart/alternative` of the text
/// and a `text/calendar; method=REPLY` part (RFC 6047 §2.4), to the organiser alone, in reply
/// to the invitation's message.
pub fn calendar_reply(original: &[u8], reply: &CalendarReply<'_>) -> Result<Posting, MimeError> {
    let to = crate::build::mailbox(&reply.to.email);
    if !to.contains('@') {
        return Err(MimeError::NoRecipients);
    }
    let decoded = crate::charset::headers_as_utf8(original);
    let original = decoded.as_deref().unwrap_or(original);
    let parent = MessageParser::new()
        .with_message_ids()
        .parse_headers(original)
        .and_then(|headers| {
            headers
                .header_values(HeaderName::MessageId)
                .filter_map(HeaderValue::as_text_list)
                .flatten()
                .map(|raw| normalize_id(raw))
                .find(|id| !id.is_empty())
        });

    let domain = reply
        .from
        .email
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| crate::build::is_domain(domain))
        .unwrap_or("localhost");
    let message_id = format!("{}@{domain}", reply.id).to_ascii_lowercase();
    let calendar = ContentType::new("text/calendar")
        .attribute("method", "REPLY")
        .attribute("charset", "UTF-8");
    let parts = vec![
        MimePart::new("text/plain", reply.text.to_owned()),
        MimePart::new(calendar, BodyPart::Text(reply.calendar.to_owned().into())),
    ];
    let alternative = ContentType::new("multipart/alternative")
        .attribute("boundary", format!("imip-{}", reply.id));

    let mut builder = MessageBuilder::new()
        .from(crate::build::mail_addr(reply.from))
        .to(crate::build::mail_addr(reply.to))
        .subject(one_line(reply.subject))
        .date(reply.at.timestamp())
        .message_id(message_id);
    if let Some(parent) = parent {
        builder = builder.in_reply_to(parent.clone()).references(parent);
    }
    let bytes = builder
        .body(MimePart::new(alternative, parts))
        .write_to_vec()
        .map_err(|err| MimeError::Unparseable(err.to_string()))?;
    Ok(Posting {
        mail_from: crate::build::mailbox(&reply.from.email),
        rcpt_to: vec![to],
        message: bytes,
    })
}

/// A subject taken from an invitation's title: its first line, so a title with a line break in
/// it cannot start a header of its own.
fn one_line(value: &str) -> String {
    value.chars().take_while(|c| !c.is_control()).collect()
}
