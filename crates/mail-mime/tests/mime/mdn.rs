//! Read receipts: seeing a request, and the RFC 8098 report that answers one.

use chrono::{DateTime, Utc};
use mail_domain::id::new_account_id;
use mail_domain::{Address, DraftId, Identity, IdentityId, IsDefault};
use mail_mime::{
    Human, OriginalHeaders, ReceiptAsk, Reporting, ReturnPath, Words, receipt, receipt_asked,
};
use mail_parser::{MessageParser, MimeHeaders};
use uuid::Uuid;

/// An original asking for a receipt, with a `Received` trail the receipt must not repeat.
const ASKING: &[u8] = b"\
Return-Path: <bounce+x1@example.test>\r\n\
Received: from mx.internal.reader.test (10.0.0.7) by imap.reader.test\r\n\
Delivered-To: me@reader.test\r\n\
Original-Recipient: rfc822;team@reader.test\r\n\
From: Ada <ada@example.test>\r\n\
To: me@reader.test\r\n\
Subject: Quarterly figures\r\n\
Date: Tue, 01 Sep 2026 09:30:00 +0000\r\n\
Message-ID: <figures-1@example.test>\r\n\
Disposition-Notification-To: Ada <ada@example.test>\r\n\
\r\n\
Please confirm you have seen these.\r\n\
";

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-02T08:00:00Z")
        .expect("literal is valid RFC 3339")
        .with_timezone(&Utc)
}

fn reader() -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: new_account_id(),
        from: Address {
            name: Some("Me".to_owned()),
            email: "me@reader.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

/// The sentences a front-end would word; written out here so the bytes can be compared.
fn words() -> Words {
    fn body(human: &Human<'_>) -> String {
        let mut out = format!(
            "This is a receipt for the message you sent to {}",
            human.reader
        );
        if let Some(date) = &human.date {
            out.push_str(&format!("\r\non {date}"));
        }
        if human.subject.is_empty() {
            out.push_str(" with no subject.\r\n");
        } else {
            out.push_str(&format!(" with the subject\r\n\"{}\".\r\n", human.subject));
        }
        out.push_str(
            "\r\nIt was displayed on the recipient's screen. That says nothing\r\n\
             about whether it was read, understood or agreed with.\r\n",
        );
        out
    }
    Words {
        subject_prefix: "Read: ",
        body,
    }
}

fn reporting(reader: &Identity, headers: OriginalHeaders) -> Reporting<'_> {
    Reporting {
        reader,
        agent: "mailo; mailo 0.1.0",
        id: DraftId::from_uuid(Uuid::from_u128(0x5eed)),
        at: at(),
        headers,
        words: words(),
    }
}

/// `raw` without the header lines starting with any of `drop`, and with `extra` added.
fn with_headers(raw: &[u8], drop: &[&str], extra: &str) -> Vec<u8> {
    let text = std::str::from_utf8(raw).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut out: Vec<&str> = head
        .split("\r\n")
        .filter(|line| !drop.iter().any(|name| line.starts_with(name)))
        .collect();
    if !extra.is_empty() {
        out.push(extra);
    }
    format!("{}\r\n\r\n{body}", out.join("\r\n")).into_bytes()
}

#[test]
fn a_request_is_seen_with_where_the_receipt_would_go() {
    assert_eq!(
        receipt_asked(ASKING),
        Some(ReceiptAsk {
            to: vec![Address {
                name: Some("Ada".to_owned()),
                email: "ada@example.test".to_owned(),
            }],
            return_path: ReturnPath::Agrees,
        })
    );
}

/// `(name, the header dropped from ASKING, the header added, what the ask reads of the
/// return path)`.
#[test]
fn receipt_asked_reads_the_return_path() {
    let cases: Vec<(&str, &str, &str, Option<ReturnPath>)> = vec![
        (
            "a message that does not ask is not asking",
            "Disposition-Notification-To",
            "",
            None,
        ),
        (
            "a request with no address is not asking",
            "Disposition-Notification-To",
            "Disposition-Notification-To: not an address",
            None,
        ),
        (
            // RFC 8098 §2.1: anyone can write the header, and a receipt confirms the mailbox is read.
            "a request to another domain than the return path is flagged",
            "Disposition-Notification-To",
            "Disposition-Notification-To: tracker@collector.test",
            Some(ReturnPath::Differs {
                return_path: "bounce+x1@example.test".to_owned(),
            }),
        ),
        (
            "with no return path there is nothing to compare",
            "Return-Path",
            "",
            Some(ReturnPath::Unknown),
        ),
        (
            "with an empty return path there is nothing to compare",
            "Return-Path",
            "Return-Path: <>",
            Some(ReturnPath::Unknown),
        ),
        (
            "the domain comparison ignores case",
            "Return-Path",
            "Return-Path: <Bounces@EXAMPLE.test>",
            Some(ReturnPath::Agrees),
        ),
    ];
    for (name, drop, extra, want) in cases {
        let raw = with_headers(ASKING, &[drop], extra);
        assert_eq!(
            receipt_asked(&raw).map(|ask| ask.return_path),
            want,
            "{name}"
        );
    }
}

#[test]
fn a_receipt_never_asks_for_a_receipt() {
    let reader = reader();
    let sent = receipt(ASKING, &reporting(&reader, OriginalHeaders::Included)).unwrap();
    // Even with the header forged onto it, a disposition report is not answered.
    let forged = with_headers(
        &sent.message,
        &[],
        "Disposition-Notification-To: me@reader.test",
    );
    assert_eq!(receipt_asked(&forged), None);
}

#[test]
fn the_receipt_is_an_rfc_8098_report_that_parses_back() {
    let reader = reader();
    let sent = receipt(ASKING, &reporting(&reader, OriginalHeaders::Included)).unwrap();
    assert_eq!(sent.mail_from, "me@reader.test");
    assert_eq!(sent.rcpt_to, vec!["ada@example.test".to_owned()]);

    let message = MessageParser::default().parse(&sent.message).unwrap();
    let ct = message.content_type().unwrap();
    assert_eq!(ct.ctype(), "multipart");
    assert_eq!(ct.subtype(), Some("report"));
    assert_eq!(
        ct.attribute("report-type"),
        Some("disposition-notification")
    );
    assert_eq!(message.subject(), Some("Read: Quarterly figures"));
    assert_eq!(
        message
            .from()
            .and_then(|f| f.first())
            .and_then(|a| a.address()),
        Some("me@reader.test")
    );
    assert_eq!(
        message
            .to()
            .and_then(|t| t.first())
            .and_then(|a| a.address()),
        Some("ada@example.test")
    );
    assert_eq!(
        message.in_reply_to().as_text(),
        Some("figures-1@example.test")
    );
    assert!(
        message
            .header_raw("Auto-Submitted")
            .is_some_and(|v| v.trim() == "auto-replied")
    );

    // Three parts under the report: what a person reads, what a machine reads, and the headers.
    let kinds: Vec<String> = message.parts[1..]
        .iter()
        .map(|part| {
            let ct = part.content_type().unwrap();
            format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or(""))
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "text/plain",
            "message/disposition-notification",
            "text/rfc822-headers"
        ]
    );

    let human = message.body_text(0).unwrap();
    assert!(human.contains("Quarterly figures"), "{human}");

    let fields = String::from_utf8(message.parts[2].contents().to_vec()).unwrap();
    assert_eq!(
        fields,
        "Reporting-UA: mailo; mailo 0.1.0\r\n\
         Original-Recipient: rfc822;team@reader.test\r\n\
         Final-Recipient: rfc822;me@reader.test\r\n\
         Original-Message-ID: <figures-1@example.test>\r\n\
         Disposition: manual-action/MDN-sent-manually; displayed\r\n"
    );

    let headers = String::from_utf8(message.parts[3].contents().to_vec()).unwrap();
    assert!(
        headers.contains("Subject: Quarterly figures\r\n"),
        "{headers}"
    );
    assert!(headers.contains("Message-ID: <figures-1@example.test>\r\n"));
    for private in ["Received", "Delivered-To", "Return-Path", "10.0.0.7"] {
        assert!(!headers.contains(private), "{private} leaked: {headers}");
    }
}

#[test]
fn the_same_inputs_build_the_same_bytes() {
    // Golden: everything in the receipt is derived from its inputs — the id names the boundary
    // and the Message-ID, `at` is the Date — so a change in the bytes is a change in behaviour.
    let reader = reader();
    let sent = receipt(ASKING, &reporting(&reader, OriginalHeaders::Omitted)).unwrap();
    let again = receipt(ASKING, &reporting(&reader, OriginalHeaders::Omitted)).unwrap();
    assert_eq!(sent, again);
    let text = String::from_utf8(sent.message).unwrap();
    let expected = "\
From: \"Me\" <me@reader.test>\r\n\
To: \"Ada\" <ada@example.test>\r\n\
Subject: Read: Quarterly figures\r\n\
Date: Wed, 2 Sep 2026 08:00:00 +0000\r\n\
Message-ID: <00000000-0000-0000-0000-000000005eed@reader.test>\r\n\
Auto-Submitted: auto-replied\r\n\
In-Reply-To: <figures-1@example.test>\r\n\
References: <figures-1@example.test>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/report; report-type=\"disposition-notification\";\r\n\
\x20boundary=\"mdn-00000000-0000-0000-0000-000000005eed\"\r\n\
\r\n\
\r\n\
--mdn-00000000-0000-0000-0000-000000005eed\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: 7bit\r\n\
\r\n\
This is a receipt for the message you sent to me@reader.test\r\n\
on Tue, 1 Sep 2026 09:30:00 +0000 with the subject\r\n\
\"Quarterly figures\".\r\n\
\r\n\
It was displayed on the recipient's screen. That says nothing\r\n\
about whether it was read, understood or agreed with.\r\n\
\r\n\
--mdn-00000000-0000-0000-0000-000000005eed\r\n\
Content-Type: message/disposition-notification\r\n\
Content-Transfer-Encoding: 7bit\r\n\
\r\n\
Reporting-UA: mailo; mailo 0.1.0\r\n\
Original-Recipient: rfc822;team@reader.test\r\n\
Final-Recipient: rfc822;me@reader.test\r\n\
Original-Message-ID: <figures-1@example.test>\r\n\
Disposition: manual-action/MDN-sent-manually; displayed\r\n\
\r\n\
--mdn-00000000-0000-0000-0000-000000005eed--\r\n";
    assert_eq!(text, expected, "\n--- got ---\n{text}");
}

#[test]
fn an_original_that_did_not_ask_gets_no_receipt() {
    let reader = reader();
    let plain = with_headers(ASKING, &["Disposition-Notification-To"], "");
    assert!(receipt(&plain, &reporting(&reader, OriginalHeaders::Omitted)).is_err());
}

#[test]
fn a_hostile_original_recipient_cannot_add_a_field() {
    let reader = reader();
    let raw = with_headers(
        ASKING,
        &["Original-Recipient"],
        "Original-Recipient: rfc822;a@b.test\u{0}Disposition: automatic-action/MDN-sent-automatically; deleted",
    );
    let sent = receipt(&raw, &reporting(&reader, OriginalHeaders::Omitted)).unwrap();
    let text = String::from_utf8_lossy(&sent.message);
    assert_eq!(text.matches("Disposition: ").count(), 1, "{text}");
}
