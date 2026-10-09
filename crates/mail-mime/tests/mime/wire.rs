//! What `build` writes is clean RFC 5322 / 2045-2049 on the wire, whatever the draft holds.
//!
//! One rule, checked over a corpus of drafts chosen to break a careless writer: every octet is
//! ASCII (non-ASCII text is in an encoded-word, RFC 2231 parameter or transfer encoding), no line
//! is over 998 octets and none is over 78 (RFC 5322 §2.1.1), and what was written reads back as
//! what the user typed. The sibling cases are the rows of the corpus, not separate tests.

use chrono::{TimeZone, Utc};
use mail_domain::id::new_account_id;
use mail_domain::{
    Address, BlobId, Draft, DraftId, Identity, IdentityId, IsDefault, PendingAttachment,
    ReceiptRequest, SendState,
};
use mail_mime::{Disclosure, build, parse};

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

fn identity() -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: new_account_id(),
        from: addr(Some("Zoë Ångström"), "me@example.test"),
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

fn draft() -> Draft {
    Draft {
        id: DraftId::generate(),
        account: new_account_id(),
        identity: IdentityId::generate(),
        to: vec![addr(Some("Bea"), "bea@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "hello".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "hi\r\n".to_owned(),
        html: None,
        attachments: Vec::new(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: mail_domain::OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap(),
    }
}

fn attached(name: &str, mime: &str) -> (PendingAttachment, (BlobId, Vec<u8>)) {
    let blob = BlobId::generate();
    (
        PendingAttachment {
            name: name.to_owned(),
            mime: mime.to_owned(),
            blob,
        },
        (blob, (0..=255u8).cycle().take(5000).collect()),
    )
}

/// `(what is special about it, the draft)`.
/// One corpus row: its name, the draft, and the attachment bytes by blob.
type Row = (&'static str, Draft, Vec<(BlobId, Vec<u8>)>);

fn corpus() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut row = |name, d: Draft, parts: Vec<(BlobId, Vec<u8>)>| rows.push((name, d, parts));

    row("plain", draft(), Vec::new());
    row(
        "a subject of 300 ASCII characters",
        Draft {
            subject: "word ".repeat(60),
            ..draft()
        },
        Vec::new(),
    );
    row(
        "a CJK subject past one encoded-word",
        Draft {
            subject: "週末のランチはどうですか。".repeat(8),
            ..draft()
        },
        Vec::new(),
    );
    row(
        "display names with accents, CJK, a comma and quotes",
        Draft {
            to: vec![
                addr(Some("José Núñez"), "jose@example.test"),
                addr(Some("王小明"), "wang@example.test"),
                addr(Some("Doe, John"), "doe@example.test"),
                addr(Some("The \"Boss\""), "boss@example.test"),
            ],
            ..draft()
        },
        Vec::new(),
    );
    row(
        "sixty recipients",
        Draft {
            to: (0..60)
                .map(|n| addr(Some("Recipient Number"), &format!("person{n}@example.test")))
                .collect(),
            ..draft()
        },
        Vec::new(),
    );
    row(
        "a text line of 5000 characters and a bare LF",
        Draft {
            text: format!("{}\nsecond line\n", "long ".repeat(1000)),
            ..draft()
        },
        Vec::new(),
    );
    row(
        "non-ASCII text and an HTML line of 4000 characters",
        Draft {
            text: "café 日本語\r\n".to_owned(),
            html: Some(format!("<p>{}</p>", "é".repeat(4000))),
            ..draft()
        },
        Vec::new(),
    );
    let files = [
        ("Jahresabschluß 2025.pdf", "application/pdf"),
        ("会議の議事録.txt", "application/octet-stream"),
        ("report \"final\"; v2.pdf", "application/pdf"),
        (
            "a very long file name that goes on and on and on and on and on and on and on 完.bin",
            "application/octet-stream",
        ),
    ];
    for (name, mime) in files {
        let (attachment, part) = attached(name, mime);
        row(
            "an attachment with a name that needs RFC 2231",
            Draft {
                attachments: vec![attachment],
                ..draft()
            },
            vec![part],
        );
    }
    rows
}

#[test]
fn every_message_is_ascii_with_lines_inside_the_limits() {
    for (name, draft, parts) in corpus() {
        let built = build(&draft, &identity(), None, &parts, Disclosure::Full).unwrap();
        assert!(built.is_ascii(), "{name}: a raw non-ASCII octet");
        for line in built.split(|b| *b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            assert!(
                line.len() <= 78,
                "{name}: a line of {} octets: {}",
                line.len(),
                String::from_utf8_lossy(line)
            );
        }
        // And every line ends CRLF, none bare (RFC 5322 §2.1).
        let bare = built
            .iter()
            .enumerate()
            .any(|(at, b)| *b == b'\n' && (at == 0 || built[at - 1] != b'\r'));
        assert!(!bare, "{name}: a bare LF");
    }
}

#[test]
fn what_the_user_typed_reads_back() {
    for (name, draft, parts) in corpus() {
        let built = build(&draft, &identity(), None, &parts, Disclosure::Full).unwrap();
        let parsed = parse(&built).unwrap();
        // Trailing blanks are not part of a subject: the corpus row of repeated words ends in one.
        assert_eq!(
            parsed.subject.trim(),
            draft.subject.trim(),
            "{name}: subject"
        );
        let from = parsed.from.expect("a From");
        assert_eq!(from.name.as_deref(), Some("Zoë Ångström"), "{name}: sender");
        assert_eq!(parsed.to.len(), draft.to.len(), "{name}: recipients");
        for (got, sent) in parsed.to.iter().zip(&draft.to) {
            assert_eq!(got.email, sent.email, "{name}");
            assert_eq!(
                got.name, sent.name,
                "{name}: display name of {}",
                sent.email
            );
        }
        for (got, (sent, (_, bytes))) in parsed
            .attachments
            .iter()
            .zip(draft.attachments.iter().zip(&parts))
        {
            assert_eq!(got.name, sent.name, "{name}: file name");
            assert_eq!(&got.bytes, bytes, "{name}: attachment bytes");
        }
        if draft.text.contains("café") {
            assert_eq!(
                parsed.text.as_deref().map(str::trim),
                Some("café 日本語"),
                "{name}"
            );
        }
    }
}

#[test]
fn boundaries_are_never_reused_between_messages() {
    let (attachment, part) = attached("a.bin", "application/octet-stream");
    let draft = Draft {
        attachments: vec![attachment],
        ..draft()
    };
    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        let built = build(
            &draft,
            &identity(),
            None,
            std::slice::from_ref(&part),
            Disclosure::Full,
        )
        .unwrap();
        let text = String::from_utf8(built).unwrap();
        let boundary = text
            .split("boundary=")
            .nth(1)
            .and_then(|rest| rest.split(['\r', ';']).next())
            .map(|value| value.trim().trim_matches('"').to_owned())
            .expect("a multipart message names its boundary");
        assert!(
            seen.insert(boundary.clone()),
            "boundary {boundary} used twice"
        );
        // RFC 2046 §5.1.1: 1 to 70 characters, and not in the content.
        assert!((1..=70).contains(&boundary.len()), "{boundary}");
        assert_eq!(
            text.matches(&format!("--{boundary}")).count(),
            3,
            "{boundary}"
        );
    }
}

#[test]
fn date_and_message_id_are_in_the_forms_rfc_5322_gives_them() {
    let built = build(&draft(), &identity(), None, &[], Disclosure::HideBlind).unwrap();
    let text = String::from_utf8(built).unwrap();
    // 2026-03-15 is a Sunday: the day name is right, not merely present (§3.3).
    assert!(
        text.contains("\r\nDate: Sun, 15 Mar 2026 12:00:00 +0000\r\n"),
        "{text}"
    );
    let id = text
        .lines()
        .find_map(|l| l.strip_prefix("Message-ID: "))
        .expect("a Message-ID");
    let inner = id
        .strip_prefix('<')
        .and_then(|i| i.strip_suffix('>'))
        .expect("angle brackets");
    let (left, right) = inner.split_once('@').expect("id-left@id-right");
    assert!(!left.is_empty() && right == "example.test", "{id}");
    assert!(
        !inner.contains(|c: char| c.is_whitespace() || c == '<' || c == '>'),
        "{id}"
    );
}

#[test]
fn a_long_references_list_folds_and_keeps_every_id() {
    use mail_domain::{
        Body, MailboxRole, Message, MessageId, MessageKey, ReadState, Star, ThreadId,
    };
    let references: Vec<String> = (0..30)
        .map(|n| format!("ref{n}-0123456789abcdef@mail.example.test"))
        .collect();
    let parent = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: new_account_id(),
        key: MessageKey::Rfc("parent@example.test".to_owned()),
        date: Utc.with_ymd_and_hms(2026, 3, 14, 12, 0, 0).unwrap(),
        from: addr(Some("Ada"), "ada@example.test"),
        reply_to: Vec::new(),
        to: vec![addr(None, "me@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Lunch".to_owned(),
        in_reply_to: None,
        references: references.clone(),
        rfc_message_id: Some("parent@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body: Body::Present {
            text: Some("original".to_owned()),
            raw: BlobId::generate(),
        },
        attachments: Vec::new(),
    };
    let built = build(
        &draft(),
        &identity(),
        Some(&parent),
        &[],
        Disclosure::HideBlind,
    )
    .unwrap();
    for line in built.split(|b| *b == b'\n') {
        assert!(line.len() <= 79, "a line of {} octets", line.len());
    }
    let parsed = parse(&built).unwrap();
    let mut want = references;
    want.push("parent@example.test".to_owned());
    assert_eq!(parsed.references, want);
    assert_eq!(parsed.in_reply_to.as_deref(), Some("parent@example.test"));
}
