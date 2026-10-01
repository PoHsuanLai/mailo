//! A reply's In-Reply-To and References are what the recipient threads on.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{
    AccountId, Address, BlobId, Body, Draft, DraftId, Identity, IdentityId, Inline, IsDefault,
    MailboxRole, Message, MessageId, MessageKey, PendingAttachment, ReadState, ReceiptRequest,
    SendState, Star, ThreadId,
};
use mail_mime::{Disclosure, MimeError, ReturnPath, build, parse, posting, receipt_asked};

struct ThreadCase {
    name: &'static str,
    /// `false` builds a new message: the parent is ignored.
    pass_parent: bool,
    references: &'static [&'static str],
    rfc_message_id: Option<&'static str>,
    expect_irt: Option<&'static str>,
    expect_refs: &'static [&'static str],
}

const THREADS: &[ThreadCase] = &[
    ThreadCase {
        name: "reply appends the parent id",
        pass_parent: true,
        references: &["root@example.test", "mid@example.test"],
        rfc_message_id: Some("parent@example.test"),
        expect_irt: Some("parent@example.test"),
        expect_refs: &[
            "root@example.test",
            "mid@example.test",
            "parent@example.test",
        ],
    },
    ThreadCase {
        name: "parent id already in references moves to the end",
        pass_parent: true,
        references: &["parent@example.test", "root@example.test"],
        rfc_message_id: Some("parent@example.test"),
        expect_irt: Some("parent@example.test"),
        expect_refs: &["root@example.test", "parent@example.test"],
    },
    ThreadCase {
        name: "parent without a message-id keeps its references",
        pass_parent: true,
        references: &["root@example.test"],
        rfc_message_id: None,
        expect_irt: None,
        expect_refs: &["root@example.test"],
    },
    ThreadCase {
        name: "parent id alone becomes references",
        pass_parent: true,
        references: &[],
        rfc_message_id: Some("parent@example.test"),
        expect_irt: Some("parent@example.test"),
        expect_refs: &["parent@example.test"],
    },
    ThreadCase {
        name: "a new message has no threading headers",
        pass_parent: false,
        references: &[],
        rfc_message_id: None,
        expect_irt: None,
        expect_refs: &[],
    },
];

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0)
        .single()
        .expect("2026-03-15T12:00:00Z is a real timestamp")
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

fn identity() -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: AccountId::generate(),
        from: addr(Some("Me"), "me@example.test"),
        reply_to: Some(addr(None, "alias@example.test")),
        signature: None,
        default: IsDefault::Default,
    }
}

fn parent(references: &[&str], rfc_message_id: Option<&str>) -> Message {
    Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: AccountId::generate(),
        key: MessageKey::Rfc("parent@example.test".to_owned()),
        date: at(),
        from: addr(Some("Ada"), "ada@example.test"),
        reply_to: Vec::new(),
        to: vec![addr(None, "me@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Lunch".to_owned(),
        in_reply_to: Some("mid@example.test".to_owned()),
        references: references.iter().map(|id| (*id).to_owned()).collect(),
        rfc_message_id: rfc_message_id.map(str::to_owned),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body: Body::Present {
            text: Some("original".to_owned()),
            raw: BlobId::generate(),
        },
        attachments: Vec::new(),
    }
}

fn draft(replying_to: Option<MessageId>, blob: Option<BlobId>) -> Draft {
    Draft {
        id: DraftId::generate(),
        account: AccountId::generate(),
        identity: IdentityId::generate(),
        to: vec![addr(Some("Bea"), "bea@example.test")],
        cc: vec![addr(None, "cara@example.test")],
        bcc: vec![addr(None, "dee@example.test")],
        subject: "Re: Lunch".to_owned(),
        in_reply_to: replying_to,
        forward_of: None,
        text: "thanks".to_owned(),
        html: Some("<p>thanks</p>".to_owned()),
        attachments: blob
            .map(|blob| PendingAttachment {
                name: "notes.bin".to_owned(),
                mime: "application/octet-stream".to_owned(),
                blob,
            })
            .into_iter()
            .collect(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: mail_domain::OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: at(),
    }
}

#[test]
fn reply_round_trips_threading_headers() {
    let me = identity();
    let blob = BlobId::generate();
    let bytes = b"abc".to_vec();
    for case in THREADS {
        let original = parent(case.references, case.rfc_message_id);
        let draft = draft(case.pass_parent.then_some(original.id), Some(blob));
        let parent_ref = case.pass_parent.then_some(&original);
        let raw = build(
            &draft,
            &me,
            parent_ref,
            &[(blob, bytes.clone())],
            Disclosure::Full,
        )
        .unwrap_or_else(|err| panic!("{}: {err}", case.name));
        let parsed = parse(&raw).unwrap_or_else(|err| panic!("{}: parse {err}", case.name));
        assert_eq!(
            parsed.in_reply_to.as_deref(),
            case.expect_irt,
            "{}",
            case.name
        );
        assert_eq!(
            parsed.references,
            case.expect_refs
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>(),
            "{}",
            case.name
        );
        if case.name == "reply appends the parent id" {
            assert_eq!(parsed.subject, "Re: Lunch", "{}", case.name);
            assert_eq!(parsed.text.as_deref().map(str::trim), Some("thanks"));
            assert!(
                parsed
                    .html
                    .as_deref()
                    .is_some_and(|html| html.contains("thanks")),
                "{}",
                case.name
            );
            assert_eq!(
                parsed.date.map(|date| date.timestamp()),
                Some(at().timestamp())
            );
            let from = parsed.from.as_ref().expect("from");
            assert_eq!(from.email, "me@example.test");
            assert_eq!(from.name.as_deref(), Some("Me"));
            assert_eq!(
                parsed
                    .reply_to
                    .iter()
                    .map(|addr| addr.email.as_str())
                    .collect::<Vec<_>>(),
                vec!["alias@example.test"]
            );
            assert_eq!(
                parsed
                    .to
                    .iter()
                    .map(|addr| addr.email.as_str())
                    .collect::<Vec<_>>(),
                vec!["bea@example.test"]
            );
            assert_eq!(
                parsed
                    .cc
                    .iter()
                    .map(|addr| addr.email.as_str())
                    .collect::<Vec<_>>(),
                vec!["cara@example.test"]
            );
            assert_eq!(
                parsed
                    .bcc
                    .iter()
                    .map(|addr| addr.email.as_str())
                    .collect::<Vec<_>>(),
                vec!["dee@example.test"]
            );
            assert_eq!(parsed.attachments.len(), 1, "{}", case.name);
            assert_eq!(parsed.attachments[0].name, "notes.bin");
            assert_eq!(parsed.attachments[0].mime, "application/octet-stream");
            assert_eq!(parsed.attachments[0].bytes, bytes);
            assert_eq!(parsed.attachments[0].inline, Inline::Attached);
            let id = parsed.rfc_message_id.as_deref().expect("message-id");
            assert!(
                id.contains(&draft.id.to_string()),
                "{id} should be derived from the draft id"
            );
        }
    }
}

#[test]
fn build_rejects_unsendable_drafts() {
    let me = identity();
    let original = parent(&[], Some("parent@example.test"));
    let blob = BlobId::generate();

    let mut empty = draft(None, None);
    empty.to.clear();
    empty.cc.clear();
    empty.bcc.clear();
    let err = build(&empty, &me, None, &[], Disclosure::Full).expect_err("no recipients");
    assert!(matches!(err, MimeError::NoRecipients), "{err:?}");

    let mut bcc_only = empty.clone();
    bcc_only.bcc.push(addr(None, "dee@example.test"));
    let raw = build(&bcc_only, &me, None, &[], Disclosure::Full).expect("bcc alone is a recipient");
    let parsed = parse(&raw).expect("bcc-only message parses");
    assert_eq!(
        parsed
            .bcc
            .iter()
            .map(|addr| addr.email.as_str())
            .collect::<Vec<_>>(),
        vec!["dee@example.test"]
    );

    let missing = draft(Some(original.id), Some(blob));
    let err =
        build(&missing, &me, Some(&original), &[], Disclosure::Full).expect_err("missing blob");
    assert!(
        matches!(err, MimeError::MissingPart(ref got) if got == &blob.to_string()),
        "{err:?}"
    );
}

#[test]
fn hostile_header_values_cannot_inject_a_header() {
    let me = identity();
    let mut draft = draft(None, Some(BlobId::generate()));
    let blob = draft.attachments[0].blob;
    draft.subject = "Hello\r\nBcc: injected@evil.test".to_owned();
    draft.to = vec![addr(
        Some("Evil\r\nBcc: also@evil.test"),
        "a@b.test\r\nBcc: injected@evil.test",
    )];
    draft.cc.clear();
    draft.bcc.clear();
    draft.attachments[0].mime = "text/plain\r\nBcc: injected@evil.test".to_owned();
    draft.attachments[0].name = "notes\r\nBcc: injected@evil.test.bin".to_owned();
    let raw = build(
        &draft,
        &me,
        None,
        &[(blob, b"abc".to_vec())],
        Disclosure::Full,
    )
    .expect("builds");
    let parsed = parse(&raw).expect("parses");
    assert!(
        parsed.bcc.is_empty(),
        "injected bcc landed in the message: {parsed:?}"
    );
    assert_eq!(parsed.to.len(), 1);
    assert_eq!(parsed.to[0].email, "a@b.test");
    assert_eq!(parsed.attachments[0].mime, "application/octet-stream");
}

/// Blind copies must reach their recipients without the other recipients learning of them.
///
/// The two halves of that sentence pull in opposite directions, and a client that gets either
/// half wrong fails quietly: strip `Bcc` from the headers but not add it to the envelope and
/// the blind recipient never receives the mail; leave it in the headers and everyone on `To`
/// sees exactly who was copied in confidence. Nothing bounces either way.
mod blind_copies {
    use super::*;

    #[test]
    fn the_wire_bytes_do_not_name_blind_recipients() {
        let me = identity();
        let draft = draft(None, None);
        let raw = build(&draft, &me, None, &[], Disclosure::HideBlind).expect("builds");
        let parsed = parse(&raw).expect("parses");

        assert!(
            parsed.bcc.is_empty(),
            "a Bcc header reached the wire: {:?}",
            parsed.bcc
        );
        // Not merely absent from the parsed struct — absent from the bytes. A header the
        // parser happens not to surface would still be delivered verbatim.
        let text = String::from_utf8_lossy(&raw).to_lowercase();
        assert!(!text.contains("bcc:"), "raw bytes still carry a Bcc header");
        assert!(
            !text.contains("dee@example.test"),
            "the blind address appears in the transmitted bytes"
        );
    }

    #[test]
    fn a_stored_copy_still_records_who_was_blind_copied() {
        // The other half of `Disclosure`. The sender's own copy must keep the record, or the
        // Sent folder forgets who actually received the message.
        let me = identity();
        let draft = draft(None, None);
        let raw = build(&draft, &me, None, &[], Disclosure::Full).expect("builds");
        let parsed = parse(&raw).expect("parses");
        assert_eq!(parsed.bcc.len(), 1, "the stored copy keeps Bcc");
    }

    #[test]
    fn the_envelope_carries_every_recipient_the_headers_hide() {
        let me = identity();
        let draft = draft(None, None);
        let post = posting(&draft, &me, None, &[]).expect("has recipients");

        assert_eq!(post.mail_from, "me@example.test");
        // To, Cc and Bcc alike: the envelope is how the blind copy is actually delivered.
        assert!(post.rcpt_to.contains(&"bea@example.test".to_owned()));
        assert!(post.rcpt_to.contains(&"cara@example.test".to_owned()));
        assert!(
            post.rcpt_to.contains(&"dee@example.test".to_owned()),
            "the blind recipient would never receive this: {:?}",
            post.rcpt_to
        );
        assert!(
            !String::from_utf8_lossy(&post.message)
                .to_lowercase()
                .contains("bcc:"),
            "posting must build with HideBlind"
        );
    }

    #[test]
    fn mail_from_is_the_identity_not_its_reply_to() {
        // The fixture identity has `reply_to: alias@example.test`. MAIL FROM is the return
        // path for bounces; pointing it at a list alias sends every bounce to the list.
        let me = identity();
        assert_eq!(me.reply_to.as_ref().unwrap().email, "alias@example.test");
        let post = posting(&draft(None, None), &me, None, &[]).expect("has recipients");
        assert_eq!(post.mail_from, "me@example.test");
    }

    #[test]
    fn someone_on_both_to_and_bcc_is_delivered_to_once() {
        let me = identity();
        let mut draft = draft(None, None);
        draft.bcc = vec![addr(None, "BEA@example.test")];
        let post = posting(&draft, &me, None, &[]).expect("has recipients");

        // Case-insensitively the same mailbox. Two RCPT TO lines mean two copies.
        assert_eq!(
            post.rcpt_to,
            vec![
                "bea@example.test".to_owned(),
                "cara@example.test".to_owned()
            ],
            "duplicate recipient survived"
        );
    }

    #[test]
    fn a_blind_only_message_still_builds_and_still_has_an_envelope() {
        // Nothing in the headers names anyone. RFC 5322 requires only Date and From, so this
        // is a valid message — and the earlier NoRecipients guard must not reject it just
        // because hiding Bcc left the header set empty.
        let me = identity();
        let mut draft = draft(None, None);
        draft.to.clear();
        draft.cc.clear();

        let post = posting(&draft, &me, None, &[]).expect("bcc alone is still a recipient");
        assert_eq!(post.rcpt_to, vec!["dee@example.test".to_owned()]);
        let parsed = parse(&post.message).expect("parses");
        assert!(parsed.to.is_empty() && parsed.cc.is_empty() && parsed.bcc.is_empty());
    }

    #[test]
    fn a_draft_with_nobody_on_it_is_refused_rather_than_sent_into_the_void() {
        let me = identity();
        let mut draft = draft(None, None);
        draft.to.clear();
        draft.cc.clear();
        draft.bcc.clear();
        assert!(matches!(
            posting(&draft, &me, None, &[]),
            Err(MimeError::NoRecipients)
        ));
    }
}

#[test]
fn a_draft_that_asks_for_a_receipt_says_so_and_one_that_does_not_does_not() {
    let me = identity();
    let mut asking = draft(None, None);
    asking.receipt = ReceiptRequest::Requested;
    let sent = posting(&asking, &me, None, &[]).unwrap();
    let text = String::from_utf8_lossy(&sent.message);
    // To the sender's own address, not the identity's `Reply-To` alias: the recipient's client
    // compares it with the return path, which is `mail_from`.
    assert!(
        text.contains("Disposition-Notification-To: \"Me\" <me@example.test>\r\n"),
        "{text}"
    );
    let with_path = [
        b"Return-Path: <".as_slice(),
        sent.mail_from.as_bytes(),
        b">\r\n",
        &sent.message,
    ]
    .concat();
    let ask = receipt_asked(&with_path).expect("the built message asks");
    assert_eq!(ask.to[0].email, "me@example.test");
    assert_eq!(ask.return_path, ReturnPath::Agrees);

    let plain = posting(&draft(None, None), &me, None, &[]).unwrap();
    assert!(!String::from_utf8_lossy(&plain.message).contains("Disposition-Notification-To"));
    assert_eq!(receipt_asked(&plain.message), None);
}

/// A message carried whole, as forwarding one as an attachment does. RFC 2046 §5.2.1 allows a
/// `message/rfc822` part only `7bit`, `8bit` or `binary`; mail-builder gives every part that is
/// not text base64, which a reader that honours the RFC shows as an opaque file.
mod enclosed_message {
    use super::*;

    struct Case {
        name: &'static str,
        carried: &'static str,
        /// What the part's body must be: the carried bytes, lines ending in CRLF.
        expect: &'static str,
        encoding: &'static str,
    }

    const CASES: &[Case] = &[
        Case {
            name: "short ASCII lines",
            carried: "From: ada@example.test\r\nSubject: Lunch\r\n\r\nThursday?\r\n",
            expect: "From: ada@example.test\r\nSubject: Lunch\r\n\r\nThursday?\r\n",
            encoding: "7bit",
        },
        Case {
            name: "8-bit bytes",
            carried: "From: ada@example.test\r\nSubject: Lunch\r\nContent-Type: text/plain; \
                      charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\nUn café ?\r\n",
            expect: "From: ada@example.test\r\nSubject: Lunch\r\nContent-Type: text/plain; \
                     charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\nUn café ?\r\n",
            encoding: "8bit",
        },
        Case {
            name: "bare line feeds, as an mbox import stores them",
            carried: "From: ada@example.test\nSubject: Lunch\n\nThursday?\n",
            expect: "From: ada@example.test\r\nSubject: Lunch\r\n\r\nThursday?\r\n",
            encoding: "7bit",
        },
    ];

    fn carrying(bytes: &[u8]) -> Vec<u8> {
        let blob = BlobId::generate();
        let mut draft = draft(None, Some(blob));
        draft.html = None;
        draft.attachments[0].name = "Lunch.eml".to_owned();
        draft.attachments[0].mime = "message/rfc822".to_owned();
        build(
            &draft,
            &identity(),
            None,
            &[(blob, bytes.to_vec())],
            Disclosure::HideBlind,
        )
        .unwrap()
    }

    /// The enclosed part's own header block and body, found by its content type.
    fn part(built: &[u8]) -> (String, Vec<u8>) {
        let parsed = mail_parser::MessageParser::default().parse(built).unwrap();
        let part = parsed
            .parts
            .iter()
            .find(|part| {
                mail_parser::MimeHeaders::content_type(*part)
                    .is_some_and(|ct| ct.ctype() == "message" && ct.subtype() == Some("rfc822"))
            })
            .expect("a message/rfc822 part");
        let header = built[part.offset_header as usize..part.offset_body as usize].to_vec();
        let body = built[part.offset_body as usize..part.offset_end as usize].to_vec();
        (String::from_utf8_lossy(&header).into_owned(), body)
    }

    #[test]
    fn it_goes_as_the_bytes_it_is_under_an_encoding_rfc_2046_allows() {
        for case in CASES {
            let built = carrying(case.carried.as_bytes());
            let (header, body) = part(&built);
            assert!(
                header.contains(&format!("Content-Transfer-Encoding: {}\r\n", case.encoding)),
                "{}: {header}",
                case.name
            );
            assert!(
                !header.to_ascii_lowercase().contains("base64"),
                "{}: {header}",
                case.name
            );
            assert!(
                header.contains("attachment") && header.contains("Lunch.eml"),
                "{}: {header}",
                case.name
            );
            assert!(
                body.starts_with(case.expect.as_bytes()),
                "{}: {:?}",
                case.name,
                String::from_utf8_lossy(&body)
            );
            let rest = &body[case.expect.len()..];
            assert!(
                rest.iter().all(|byte| matches!(byte, b'\r' | b'\n')),
                "{}: more than the message in the part: {:?}",
                case.name,
                String::from_utf8_lossy(rest)
            );
        }
    }

    #[test]
    fn it_parses_back_to_the_message_it_carried() {
        for case in CASES {
            let built = carrying(case.carried.as_bytes());
            let outer = parse(&built).unwrap();
            assert_eq!(outer.attachments.len(), 1, "{}", case.name);
            assert_eq!(outer.attachments[0].mime, "message/rfc822", "{}", case.name);
            let inner = parse(&outer.attachments[0].bytes).unwrap();
            // What was carried, in its canonical CRLF form (the case's `expect`).
            let original = parse(case.expect.as_bytes()).unwrap();
            assert_eq!(inner.subject, original.subject, "{}", case.name);
            assert_eq!(inner.from, original.from, "{}", case.name);
            assert_eq!(inner.text, original.text, "{}", case.name);
        }
    }

    #[test]
    fn a_line_smtp_cannot_carry_is_labelled_binary_rather_than_claimed_to_be_7bit() {
        let long = format!(
            "From: ada@example.test\r\nSubject: Long\r\n\r\n{}\r\n",
            "x".repeat(1200)
        );
        let (header, _) = part(&carrying(long.as_bytes()));
        assert!(
            header.contains("Content-Transfer-Encoding: binary\r\n"),
            "{header}"
        );
    }
}
