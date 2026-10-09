//! Finding the calendar object in an invitation message, and wrapping an answer (RFC 6047).

use chrono::{DateTime, Utc};
use mail_domain::{Address, DraftId};
use mail_mime::{CalendarReply, calendar_part, calendar_reply};
use mail_parser::{MessageParser, MimeHeaders, PartType};

const EVENT: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:u1\r\n\
                     SUMMARY:Caf\u{e9} chat\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-24T08:30:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// An invitation as calendar servers send one: text, HTML and the calendar as alternatives,
/// and the same object again as an `invite.ics` attachment.
fn invitation() -> Vec<u8> {
    format!(
        "From: Ada <ada@example.test>\r\n\
         To: me@example.test\r\n\
         Subject: Invitation: Caf\u{e9} chat\r\n\
         Message-ID: <invite-1@example.test>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
         \r\n\
         --outer\r\n\
         Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
         \r\n\
         --alt\r\n\
         Content-Type: text/plain; charset=UTF-8\r\n\
         \r\n\
         You have been invited.\r\n\
         --alt\r\n\
         Content-Type: text/html; charset=UTF-8\r\n\
         \r\n\
         <p>You have been invited.</p>\r\n\
         --alt\r\n\
         Content-Type: text/calendar; charset=UTF-8; method=REQUEST\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {}\r\n\
         --alt--\r\n\
         --outer\r\n\
         Content-Type: application/ics; name=\"invite.ics\"\r\n\
         Content-Disposition: attachment; filename=\"invite.ics\"\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {}\r\n\
         --outer--\r\n",
        b64(EVENT.replace("u1", "from-alternative").as_bytes()),
        b64(EVENT.replace("u1", "from-attachment").as_bytes()),
    )
    .into_bytes()
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn the_text_calendar_alternative_is_the_invitation() {
    let part = calendar_part(&invitation()).unwrap();
    assert_eq!(part.method.as_deref(), Some("REQUEST"));
    assert!(part.text.contains("UID:from-alternative"), "{}", part.text);
    assert!(
        part.text.contains("SUMMARY:Caf\u{e9} chat"),
        "decoded as UTF-8"
    );
}

#[test]
fn an_ics_attachment_is_read_when_there_is_no_calendar_part() {
    const CASES: &[&str] = &[
        "Content-Type: application/ics; name=\"invite.ics\"\r\n",
        "Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"Meeting.ICS\"\r\n",
    ];
    for header in CASES {
        let raw = format!(
            "From: a@example.test\r\nTo: me@example.test\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
             --b\r\n{header}Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--b--\r\n",
            b64(format!("\u{feff}{EVENT}").as_bytes())
        );
        let part = calendar_part(raw.as_bytes()).unwrap_or_else(|| panic!("{header}"));
        assert_eq!(part.method, None, "{header}");
        assert!(
            part.text.starts_with("BEGIN:VCALENDAR"),
            "BOM dropped: {header}"
        );
    }
}

#[test]
fn a_message_with_no_calendar_or_only_a_forwarded_one_has_none() {
    let plain = b"From: a@example.test\r\nSubject: s\r\n\r\nhello\r\n";
    assert_eq!(calendar_part(plain), None);
    let forwarded = format!(
        "From: a@example.test\r\nSubject: Fwd\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nFYI\r\n\
         --b\r\nContent-Type: message/rfc822\r\n\r\n{}\r\n--b--\r\n",
        String::from_utf8(invitation()).unwrap()
    );
    assert_eq!(calendar_part(forwarded.as_bytes()), None);
    assert_eq!(calendar_part(b""), None);
}

#[test]
fn an_answer_is_calendar_and_text_alternatives_to_the_organiser_alone() {
    let me = Address {
        name: Some("Me".into()),
        email: "me@example.test".into(),
    };
    let organiser = Address {
        name: Some("Ada".into()),
        email: "ada@example.test".into(),
    };
    let calendar = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nEND:VCALENDAR\r\n";
    let id = DraftId::generate();
    let post = calendar_reply(
        &invitation(),
        &CalendarReply {
            from: &me,
            to: &organiser,
            subject: "Accepted: Caf\u{e9} chat\r\nBcc: victim@example.test",
            text: "Me has accepted.",
            calendar,
            id,
            at: at(),
        },
    )
    .unwrap();
    assert_eq!(post.mail_from, "me@example.test");
    assert_eq!(post.rcpt_to, vec!["ada@example.test".to_owned()]);

    let message = MessageParser::default().parse(&post.message).unwrap();
    assert_eq!(message.subject(), Some("Accepted: Caf\u{e9} chat"));
    assert!(
        message.bcc().is_none(),
        "the title's line break made no header"
    );
    assert_eq!(
        message.in_reply_to().as_text(),
        Some("invite-1@example.test")
    );
    let top = message.content_type().unwrap();
    assert_eq!(top.subtype(), Some("alternative"));
    let leaves: Vec<_> = message
        .parts
        .iter()
        .filter(|p| !matches!(p.body, PartType::Multipart(_)))
        .collect();
    assert_eq!(leaves.len(), 2);
    let cal = leaves[1].content_type().unwrap();
    assert_eq!((cal.ctype(), cal.subtype()), ("text", Some("calendar")));
    assert_eq!(cal.attribute("method"), Some("REPLY"));
    assert!(
        cal.attribute("charset")
            .is_some_and(|c| c.eq_ignore_ascii_case("utf-8"))
    );
    assert_eq!(leaves[1].text_contents(), Some(calendar));
    assert_eq!(leaves[0].text_contents(), Some("Me has accepted."));

    // Deterministic: the same inputs build the same bytes.
    let again = calendar_reply(
        &invitation(),
        &CalendarReply {
            from: &me,
            to: &organiser,
            subject: "Accepted: Caf\u{e9} chat\r\nBcc: victim@example.test",
            text: "Me has accepted.",
            calendar,
            id,
            at: at(),
        },
    )
    .unwrap();
    assert_eq!(again.message, post.message);
}
