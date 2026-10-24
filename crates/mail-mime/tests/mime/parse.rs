//! Malformed mail is a fact to record. Only bytes that are not a message are an error.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::Inline;
use mail_mime::{MimeError, Parsed, parse};

const NO_ID: &[u8] = b"\
From: ada@example.test\r\n\
To: bea@example.test\r\n\
Subject: bare\r\n\
\r\n\
just text\r\n\
";

const BAD_DATE: &[u8] = b"\
From: ada@example.test\r\n\
Date: not-a-date\r\n\
Subject: when\r\n\
\r\n\
body\r\n\
";

const QP: &[u8] = b"\
From: ada@example.test\r\n\
Subject: qp\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
caf=C3=A9\r\n\
";

const B64: &[u8] = b"\
From: ada@example.test\r\n\
Subject: file\r\n\
MIME-Version: 1.0\r\n\
Content-Type: application/pdf; name=\"note.pdf\"\r\n\
Content-Disposition: attachment; filename=\"note.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
aGVsbG8=\r\n\
";

const ALT: &[u8] = b"\
From: ada@example.test\r\n\
Subject: alt\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
\r\n\
--alt\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
\r\n\
plain side\r\n\
--alt\r\n\
Content-Type: text/html; charset=\"utf-8\"\r\n\
\r\n\
<p>html side</p>\r\n\
--alt--\r\n\
";

const NESTED: &[u8] = b"\
From: Ada <ada@example.test>\r\n\
Reply-To: list@example.test\r\n\
To: Bea <bea@example.test>\r\n\
Cc: cara@example.test\r\n\
Bcc: dee@example.test\r\n\
Subject: nested\r\n\
Date: Sun, 15 Mar 2026 12:00:00 +0000\r\n\
Message-ID: <nested@example.test>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
\r\n\
--mix\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
\r\n\
--alt\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
\r\n\
plain body\r\n\
--alt\r\n\
Content-Type: text/html; charset=\"utf-8\"\r\n\
\r\n\
<p>html body</p>\r\n\
--alt--\r\n\
--mix\r\n\
Content-Type: application/octet-stream\r\n\
Content-Disposition: attachment; filename=\"note.bin\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
aGVsbG8=\r\n\
--mix--\r\n\
";

const ENCODED: &[u8] = b"\
From: =?utf-8?q?Ada_Lovelace?= <ada@example.test>\r\n\
To: bea@example.test\r\n\
Subject: =?utf-8?q?caf=C3=A9?=\r\n\
Message-ID: <encoded@example.test>\r\n\
\r\n\
hi\r\n\
";

const EIGHT_BIT: &[u8] = b"\
From: ada@example.test\r\n\
Subject: caf\xc3\xa9\r\n\
\r\n\
body\r\n\
";

const BAD_CHARSET: &[u8] = b"\
From: ada@example.test\r\n\
Subject: charset\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=\"not-a-charset\"\r\n\
\r\n\
still readable\r\n\
";

const TRUNCATED: &[u8] = b"\
From: ada@example.test\r\n\
Subject: cut\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"cut\"\r\n\
\r\n\
--cut\r\n\
Content-Type: text/plain; charset=\"utf-8\"\r\n\
\r\n\
hello from a cut-off part\r\n\
";

const THREAD: &[u8] = b"\
From: ada@example.test\r\n\
Subject: thread\r\n\
Message-ID: <AbC@Example.TEST>\r\n\
In-Reply-To: <Old@Example.TEST> <New@Example.TEST>\r\n\
References: <Root@Example.TEST> <Old@Example.TEST> <root@example.test>\r\n\
\r\n\
body\r\n\
";

const CID: &[u8] = b"\
From: ada@example.test\r\n\
Subject: logo\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\
\r\n\
--rel\r\n\
Content-Type: text/html; charset=\"utf-8\"\r\n\
\r\n\
<p><img src=\"cid:logo@mail.test\"></p>\r\n\
--rel\r\n\
Content-Type: image/png\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-ID: <logo@mail.test>\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
\r\n\
aGVsbG8=\r\n\
--rel--\r\n\
";

type ParseCase = (&'static str, &'static [u8], fn(&Parsed));

const CASES: &[ParseCase] = &[
    ("no message-id", NO_ID, check_no_id),
    ("unparseable date", BAD_DATE, check_bad_date),
    ("quoted-printable", QP, check_qp),
    ("base64 attachment", B64, check_b64),
    ("multipart alternative", ALT, check_alt),
    ("nested multipart", NESTED, check_nested),
    ("rfc 2047 encoded-word", ENCODED, check_encoded),
    ("8-bit header", EIGHT_BIT, check_eight_bit),
    ("mislabelled charset", BAD_CHARSET, check_charset),
    ("truncated multipart", TRUNCATED, check_truncated),
    ("normalized ids", THREAD, check_thread),
    ("cid image", CID, check_cid),
];

const UNPARSEABLE: &[(&str, &[u8])] = &[
    ("empty", b""),
    ("nuls", b"\0\0\0"),
    ("prose", b"this is not a message\n"),
];

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0)
        .single()
        .expect("2026-03-15T12:00:00Z is a real timestamp")
}

fn text(parsed: &Parsed) -> &str {
    parsed.text.as_deref().map(str::trim).unwrap_or("")
}

fn check_no_id(parsed: &Parsed) {
    assert_eq!(parsed.rfc_message_id, None);
    assert_eq!(parsed.date, None);
    assert_eq!(parsed.subject, "bare");
    assert_eq!(email(&parsed.from), Some("ada@example.test"));
    assert_eq!(emails(&parsed.to), vec!["bea@example.test"]);
    assert_eq!(text(parsed), "just text");
    assert_eq!(parsed.html, None);
    assert!(parsed.references.is_empty());
    assert_eq!(parsed.in_reply_to, None);
}

fn check_bad_date(parsed: &Parsed) {
    assert_eq!(
        parsed.date, None,
        "an unparseable Date is absent, not an error"
    );
    assert_eq!(parsed.subject, "when");
    assert_eq!(text(parsed), "body");
}

fn check_qp(parsed: &Parsed) {
    assert_eq!(text(parsed), "café");
    assert_eq!(parsed.html, None);
}

fn check_b64(parsed: &Parsed) {
    assert_eq!(parsed.attachments.len(), 1);
    let part = &parsed.attachments[0];
    assert_eq!(part.name, "note.pdf");
    assert_eq!(part.mime, "application/pdf");
    assert_eq!(part.bytes, b"hello");
    assert_eq!(part.inline, Inline::Attached);
    assert_eq!(parsed.text, None);
}

fn check_alt(parsed: &Parsed) {
    assert_eq!(text(parsed), "plain side");
    let html = parsed.html.as_deref().expect("html part");
    assert!(html.contains("<p>"), "{html:?}");
    assert!(html.contains("html side"), "{html:?}");
    assert!(parsed.attachments.is_empty());
}

fn check_nested(parsed: &Parsed) {
    assert_eq!(
        parsed.rfc_message_id.as_deref(),
        Some("nested@example.test")
    );
    assert_eq!(parsed.date, Some(at()));
    assert_eq!(parsed.subject, "nested");
    assert_eq!(name_email(&parsed.from), Some(("Ada", "ada@example.test")));
    assert_eq!(emails(&parsed.reply_to), vec!["list@example.test"]);
    assert_eq!(
        name_email_list(&parsed.to),
        vec![("Bea", "bea@example.test")]
    );
    assert_eq!(emails(&parsed.cc), vec!["cara@example.test"]);
    assert_eq!(emails(&parsed.bcc), vec!["dee@example.test"]);
    assert_eq!(text(parsed), "plain body");
    let html = parsed.html.as_deref().expect("html part");
    assert!(html.contains("html body"), "{html:?}");
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].name, "note.bin");
    assert_eq!(parsed.attachments[0].mime, "application/octet-stream");
    assert_eq!(parsed.attachments[0].bytes, b"hello");
}

fn check_encoded(parsed: &Parsed) {
    assert_eq!(parsed.subject, "café");
    assert_eq!(
        name_email(&parsed.from),
        Some(("Ada Lovelace", "ada@example.test"))
    );
    assert_eq!(
        parsed.rfc_message_id.as_deref(),
        Some("encoded@example.test")
    );
}

fn check_eight_bit(parsed: &Parsed) {
    assert_eq!(parsed.subject, "café");
    assert_eq!(text(parsed), "body");
}

fn check_charset(parsed: &Parsed) {
    assert_eq!(parsed.subject, "charset");
    assert_eq!(text(parsed), "still readable");
}

fn check_truncated(parsed: &Parsed) {
    assert!(
        text(parsed).contains("hello from a cut-off part"),
        "{:?}",
        parsed.text
    );
}

fn check_thread(parsed: &Parsed) {
    assert_eq!(parsed.rfc_message_id.as_deref(), Some("abc@example.test"));
    assert_eq!(parsed.in_reply_to.as_deref(), Some("new@example.test"));
    assert_eq!(
        parsed.references,
        vec![
            "root@example.test".to_owned(),
            "old@example.test".to_owned(),
        ]
    );
}

fn check_cid(parsed: &Parsed) {
    let html = parsed.html.as_deref().expect("html part");
    assert!(html.contains("cid:logo@mail.test"), "{html:?}");
    assert_eq!(parsed.attachments.len(), 1);
    let part = &parsed.attachments[0];
    assert_eq!(part.name, "logo.png");
    assert_eq!(part.mime, "image/png");
    assert_eq!(part.bytes, b"hello");
    assert_eq!(
        part.inline,
        Inline::Embedded {
            cid: "logo@mail.test".to_owned(),
        }
    );
}

fn email(addr: &Option<mail_domain::Address>) -> Option<&str> {
    addr.as_ref().map(|addr| addr.email.as_str())
}

fn emails(addrs: &[mail_domain::Address]) -> Vec<&str> {
    addrs.iter().map(|addr| addr.email.as_str()).collect()
}

fn name_email(addr: &Option<mail_domain::Address>) -> Option<(&str, &str)> {
    let addr = addr.as_ref()?;
    Some((addr.name.as_deref().unwrap_or(""), addr.email.as_str()))
}

fn name_email_list(addrs: &[mail_domain::Address]) -> Vec<(&str, &str)> {
    addrs
        .iter()
        .map(|addr| (addr.name.as_deref().unwrap_or(""), addr.email.as_str()))
        .collect()
}

#[test]
fn parse_records_malformed_mail() {
    for (name, raw, check) in CASES {
        let parsed = parse(raw).unwrap_or_else(|err| panic!("{name}: {err}"));
        check(&parsed);
    }
}

#[test]
fn bytes_that_are_not_a_message_are_unparseable() {
    for (name, raw) in UNPARSEABLE {
        let err = parse(raw).expect_err(name);
        assert!(matches!(err, MimeError::Unparseable(_)), "{name}: {err:?}");
    }
}

/// `format` and `delsp` of the text part `text` came from, including when an
/// earlier text/plain in the tree says something else.
#[test]
fn flowed_is_read_from_the_text_part_that_text_came_from() {
    use mail_mime::{Delsp, Flowed};

    let cases: &[(&str, &[u8], &str, Flowed)] = &[
        (
            "a single part that flows, deleting the space",
            b"From: ada@example.test\r\n\
              Subject: s\r\n\
              MIME-Version: 1.0\r\n\
              Content-Type: text/plain; charset=utf-8; format=flowed; delsp=yes\r\n\
              \r\n\
              hello \r\n\
              there\r\n",
            "hello",
            Flowed::Flowed { delsp: Delsp::Yes },
        ),
        (
            "flowed without delsp keeps the parameter false",
            b"From: ada@example.test\r\n\
              Subject: s\r\n\
              MIME-Version: 1.0\r\n\
              Content-Type: text/plain; charset=utf-8; format=flowed\r\n\
              \r\n\
              hello \r\n\
              there\r\n",
            "hello",
            Flowed::Flowed { delsp: Delsp::No },
        ),
        (
            "absent format is fixed",
            b"From: ada@example.test\r\nSubject: s\r\n\r\nhello\r\n",
            "hello",
            Flowed::Fixed,
        ),
        (
            "the first text/plain is an attachment that flows; the body does not",
            b"From: ada@example.test\r\n\
              Subject: s\r\n\
              MIME-Version: 1.0\r\n\
              Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
              \r\n\
              --mix\r\n\
              Content-Type: text/plain; charset=utf-8; format=flowed; delsp=yes\r\n\
              Content-Disposition: attachment; filename=\"notes.txt\"\r\n\
              \r\n\
              attachment flowed\r\n\
              --mix\r\n\
              Content-Type: text/plain; charset=utf-8; format=fixed\r\n\
              \r\n\
              body fixed\r\n\
              --mix--\r\n",
            "body fixed",
            Flowed::Fixed,
        ),
        (
            "the body flows; an earlier attached text/plain does not",
            b"From: ada@example.test\r\n\
              Subject: s\r\n\
              MIME-Version: 1.0\r\n\
              Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
              \r\n\
              --mix\r\n\
              Content-Type: text/plain; charset=utf-8; format=fixed\r\n\
              Content-Disposition: attachment; filename=\"notes.txt\"\r\n\
              \r\n\
              attachment fixed\r\n\
              --mix\r\n\
              Content-Type: text/plain; charset=utf-8; format=flowed; delsp=yes\r\n\
              \r\n\
              body flowed\r\n\
              --mix--\r\n",
            "body flowed",
            Flowed::Flowed { delsp: Delsp::Yes },
        ),
    ];

    let mut failures = Vec::new();
    for (name, raw, needle, flowed) in cases {
        let parsed = parse(raw).unwrap_or_else(|err| panic!("{name}: {err}"));
        let text = parsed.text.as_deref().unwrap_or("");
        if !text.contains(needle) || parsed.flowed != *flowed {
            failures.push(format!(
                "{name}: text {text:?} flowed {:?} (want {flowed:?}, text containing {needle:?})",
                parsed.flowed
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A reconstructed part carries its own `N.MIME` header. `format=flowed` on that
/// header has to survive being written back into the document.
#[test]
fn a_reconstructed_text_part_keeps_its_flowed_parameters() {
    use mail_domain::PartTree;
    use mail_mime::{Delsp, Flowed, parse_reconstructed, reconstruct};
    use std::collections::HashMap;

    let header = "From: ada@example.test\r\n\
        Subject: re\r\n\
        MIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
        \r\n";
    // First text/plain in the tree is an attachment whose header says fixed.
    // The body text part's own header says flowed. The body wins.
    let attach_mime = "Content-Type: text/plain; charset=utf-8; format=fixed\r\n\
        Content-Disposition: attachment; filename=\"notes.txt\"\r\n\
        \r\n";
    let attach = "attachment fixed\r\n";
    let plain_mime = "Content-Type: text/plain; charset=utf-8; format=flowed; delsp=yes\r\n\r\n";
    let plain = "hello \r\nthere\r\n";

    let tree = PartTree::Multipart {
        section: mail_domain::Section::root(),
        subtype: "mixed".to_owned(),
        boundary: "mix".to_owned(),
        parts: vec![
            PartTree::Leaf {
                section: "1".parse().unwrap(),
                mime: "text/plain".into(),
                octets: attach.len() as u64,
                attachment: true,
            },
            PartTree::Leaf {
                section: "2".parse().unwrap(),
                mime: "text/plain".into(),
                octets: plain.len() as u64,
                attachment: false,
            },
        ],
    };
    let mut fetched = HashMap::new();
    fetched.insert("HEADER".into(), header.as_bytes().to_vec());
    fetched.insert("1.MIME".into(), attach_mime.as_bytes().to_vec());
    fetched.insert("1".into(), attach.as_bytes().to_vec());
    fetched.insert("2.MIME".into(), plain_mime.as_bytes().to_vec());
    fetched.insert("2".into(), plain.as_bytes().to_vec());

    let raw = reconstruct(&tree, &fetched).expect("the sections rebuild a message");
    let parsed = parse_reconstructed(&raw).expect("the rebuilt message parses");
    let text = parsed.text.as_deref().unwrap_or("");
    assert!(
        text.contains("hello") && !text.contains("attachment"),
        "text came from the attachment, not the body: {text:?}"
    );
    assert_eq!(parsed.flowed, Flowed::Flowed { delsp: Delsp::Yes });
}

/// An invitation's calendar alternative is the message in a calendar's form, not a file: no
/// attachment. A calendar the sender attached stays one, and a part sent without a name is
/// called as a person would call it, never by its position.
#[test]
fn an_invitations_calendar_is_no_attachment_and_an_unnamed_part_has_a_plain_name() {
    let invite = b"From: ada@example.test\r\nSubject: Invitation\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n--alt\r\n\
Content-Type: text/plain; charset=UTF-8\r\n\r\nYou are invited.\r\n--alt\r\n\
Content-Type: text/calendar; charset=UTF-8; method=REQUEST\r\n\r\n\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nEND:VCALENDAR\r\n--alt--\r\n";
    let parsed = parse(invite).expect("parsed");
    assert!(parsed.attachments.is_empty(), "{:?}", parsed.attachments);
    assert!(mail_mime::calendar_part(invite).is_some());

    let attached = b"From: ada@example.test\r\nSubject: Event\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\n\
Content-Type: text/plain\r\n\r\nThe event.\r\n--mix\r\n\
Content-Type: text/calendar\r\nContent-Disposition: attachment\r\n\r\n\
BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--mix\r\n\
Content-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--mix--\r\n";
    let parsed = parse(attached).expect("parsed");
    let names: Vec<&str> = parsed
        .attachments
        .iter()
        .map(|part| part.name.as_str())
        .collect();
    assert_eq!(names, ["Attachment 1.ics", "Attachment 2.pdf"]);
}

mod any_bytes {
    use mail_mime::parse;
    use proptest::prelude::*;

    /// Bytes shaped like mail often enough to reach past the header: header lines, boundaries,
    /// encodings and encoded words, with arbitrary bytes between them.
    fn mailish() -> impl Strategy<Value = Vec<u8>> {
        let piece = prop_oneof![
            prop::sample::select(vec![
                &b"From: Ada <ada@example.test>\r\n"[..],
                b"Subject: =?utf-8?b?Y2Fmw6k=?= =?iso-2022-jp?q?=1B$B?=\r\n",
                b"MIME-Version: 1.0\r\n",
                b"Content-Type: multipart/mixed; boundary=\"b\"\r\n",
                b"Content-Type: multipart/alternative; boundary=b\r\n",
                b"Content-Type: text/plain; charset=big5; format=flowed; delsp=yes\r\n",
                b"Content-Type: text/html; charset=\"x-unknown\"\r\n",
                b"Content-Type: message/rfc822\r\n",
                b"Content-Type: image/png; name*=utf-8''%E6%97%A5\r\n",
                b"Content-Transfer-Encoding: base64\r\n",
                b"Content-Transfer-Encoding: quoted-printable\r\n",
                b"Content-Disposition: inline; filename=\"a.png\"\r\n",
                b"Content-ID: <logo@x>\r\n",
                b"Date: Sun, 15 Mar 2026 12:00:00 +0000\r\n",
                b"References: <a@x> <b@x\r\n",
                b"--b\r\n",
                b"--b--\r\n",
                b"\r\n",
                b" folded\r\n",
                b"=C3=A9=\r\n",
                b"aGVsbG8=\r\n",
            ])
            .prop_map(<[u8]>::to_vec),
            prop::collection::vec(any::<u8>(), 0..32),
        ];
        prop::collection::vec(piece, 0..24).prop_map(|pieces| pieces.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// A message is bytes from a stranger: whatever they are, reading them is a value or an
        /// error, never a panic.
        #[test]
        fn parsing_any_bytes_never_panics(
            raw in prop_oneof![prop::collection::vec(any::<u8>(), 0..2048), mailish()]
        ) {
            // Either answer is right; only a panic is wrong.
            let _ = parse(&raw);
        }
    }
}
