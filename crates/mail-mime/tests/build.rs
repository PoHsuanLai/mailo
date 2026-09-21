//! A reply's In-Reply-To and References are what the recipient threads on.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{
    AccountId, Address, BlobId, Body, Draft, DraftId, Identity, IdentityId, Inline, IsDefault,
    MailboxRole, Message, MessageId, MessageKey, PendingAttachment, ReadState, SendState, Star,
    ThreadId,
};
use mail_mime::{MimeError, build, parse};

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
        body: Body {
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
        let raw = build(&draft, &me, parent_ref, &[(blob, bytes.clone())])
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
    let err = build(&empty, &me, None, &[]).expect_err("no recipients");
    assert!(matches!(err, MimeError::NoRecipients), "{err:?}");

    let mut bcc_only = empty.clone();
    bcc_only.bcc.push(addr(None, "dee@example.test"));
    let raw = build(&bcc_only, &me, None, &[]).expect("bcc alone is a recipient");
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
    let err = build(&missing, &me, Some(&original), &[]).expect_err("missing blob");
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
    let raw = build(&draft, &me, None, &[(blob, b"abc".to_vec())]).expect("builds");
    let parsed = parse(&raw).expect("parses");
    assert!(
        parsed.bcc.is_empty(),
        "injected bcc landed in the message: {parsed:?}"
    );
    assert_eq!(parsed.to.len(), 1);
    assert_eq!(parsed.to[0].email, "a@b.test");
    assert_eq!(parsed.attachments[0].mime, "application/octet-stream");
}
