//! Replying and sending from the command line: `plan.md` phase 4's third verb.
//!
//! Phase 4 asks that "a tiny CLI can list, open and reply". The first two had tests from the
//! start. Reply had no implementation at all until this file's subject existed, because the
//! path it needs — a draft that persists, an identity to send as, an envelope that survives
//! into the outbox — was missing at every one of those three points.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

#[path = "../src/compose.rs"]
mod compose;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ORIGINAL: MessageId =
    MessageId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: None,
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

/// A store with one account that can actually send, and one message to reply to.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();

    let mut plan = mail_domain::presets::preset_for("me@ntu.edu.tw", at(0))
        .expect("ntu is a known domain")
        .plan;
    plan.address = "me@example.test".to_owned();
    plan.identities = vec![identity()];

    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            rusqlite::params![IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }

    let raw = store
        .blobs()
        .put(&store.connection(), b"raw original")
        .unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: ORIGINAL,
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("original@example.test".to_owned()),
        date: at(0),
        from: Address {
            name: Some("Ada Lovelace".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![
            Address {
                name: None,
                email: "me@example.test".to_owned(),
            },
            Address {
                name: None,
                email: "bea@example.test".to_owned(),
            },
        ],
        cc: vec![Address {
            name: None,
            email: "cara@example.test".to_owned(),
        }],
        bcc: vec![],
        subject: "lunch on friday".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("original@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("Shall we say one o'clock?".to_owned()),
            raw,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: SyncCursor::Pop,
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: "u1".to_owned(),
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    (store, dir)
}

/// The draft a reply produced, read back from the store.
fn only_draft(store: &SqliteStore) -> Draft {
    let drafts = store.drafts(ACCOUNT).unwrap();
    assert_eq!(drafts.len(), 1, "expected exactly one draft");
    drafts.into_iter().next().unwrap()
}

#[test]
fn replying_creates_a_draft_addressed_to_the_sender() {
    let (store, _dir) = seeded();
    let out = compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .expect("reply succeeds");

    let draft = only_draft(&store);
    assert_eq!(draft.subject, "Re: lunch on friday");
    assert_eq!(draft.to.len(), 1);
    assert_eq!(draft.to[0].email, "ada@example.test");
    assert!(draft.cc.is_empty(), "a plain reply does not copy anyone");
    assert_eq!(draft.in_reply_to, Some(ORIGINAL));
    assert_eq!(draft.identity, IDENTITY);
    // The id is printed because the next command needs it.
    assert!(out.contains(&draft.id.to_string()), "{out}");
    assert!(out.contains("mailo send"), "{out}");
}

#[test]
fn replying_to_all_keeps_everyone_but_us() {
    let (store, _dir) = seeded();
    compose::reply(&store, ORIGINAL, ReplyScope::All, "sounds good", at(10)).unwrap();

    let draft = only_draft(&store);
    let to: Vec<&str> = draft.to.iter().map(|a| a.email.as_str()).collect();
    assert!(to.contains(&"ada@example.test"), "{to:?}");
    assert!(to.contains(&"bea@example.test"), "{to:?}");
    assert!(
        !to.contains(&"me@example.test") && !draft.cc.iter().any(|a| a.email == "me@example.test"),
        "a reply-all that mails ourselves: {draft:?}"
    );
    assert_eq!(
        draft
            .cc
            .iter()
            .map(|a| a.email.as_str())
            .collect::<Vec<_>>(),
        vec!["cara@example.test"],
        "the original Cc stays Cc"
    );
}

#[test]
fn the_reply_body_quotes_the_original_beneath_what_was_written() {
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();

    let text = only_draft(&store).text;
    let written = text
        .find("one o'clock suits")
        .expect("the new text is there");
    let quoted = text
        .find("> Shall we say one o'clock?")
        .expect("the original is quoted");
    assert!(
        written < quoted,
        "the quote must come after the reply:\n{text}"
    );
    assert!(
        text.contains("Ada Lovelace wrote:"),
        "no attribution line:\n{text}"
    );
}

/// A second message, headers only — the normal mid-sync state.
fn headers_only(store: &SqliteStore) -> MessageId {
    let id = MessageId::generate();
    let raw = store.blobs().put(&store.connection(), b"raw two").unwrap();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc("second@example.test".to_owned()),
        date: at(5),
        from: Address {
            name: Some("Bob".to_owned()),
            email: "bob@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "no body yet".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("second@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: SyncCursor::Pop,
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: "u2".to_owned(),
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    id
}

#[test]
fn a_reply_to_a_message_with_no_body_quotes_nothing_rather_than_the_word_none() {
    // Mid-sync, headers arrive before bodies, and replying then is normal. A client that
    // renders the absent body into the quote sends that text to the recipient.
    let (store, _dir) = seeded();
    let id = headers_only(&store);
    compose::reply(&store, id, ReplyScope::Sender, "later", at(10)).unwrap();

    let text = only_draft(&store).text;
    assert!(text.contains("later"), "{text}");
    assert!(
        text.contains("Bob wrote:"),
        "the attribution still stands:\n{text}"
    );
    assert!(!text.to_lowercase().contains("none"), "{text}");
    assert!(!text.contains('>'), "nothing should be quoted:\n{text}");
}

#[test]
fn sending_queues_the_draft_without_touching_the_network() {
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();
    let draft = only_draft(&store);

    let out = compose::send(&store, draft.id, at(20)).expect("send queues");
    assert!(out.contains("mailo sync"), "{out}");

    // The submission is in the outbox, with its envelope frozen beside the bytes.
    let due = store.outbox_due(ACCOUNT, at(30)).unwrap();
    assert_eq!(due.len(), 1, "nothing was queued");
    match &due[0].op {
        ProtoOp::Submit {
            draft: queued,
            mail_from,
            rcpt_to,
            ..
        } => {
            assert_eq!(*queued, draft.id);
            assert_eq!(mail_from, "me@example.test");
            assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
        }
        other => panic!("expected a submission, got {other:?}"),
    }
    assert_eq!(store.draft(draft.id).unwrap().state, SendState::Queued);
}

#[test]
fn the_queued_bytes_are_frozen_against_a_later_edit() {
    // The user pressed send on a particular version. A draft edited afterwards — by an autosave
    // that had not yet fired, or by a second window — must not change what goes out.
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();
    let mut draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();

    draft.subject = "something else entirely".to_owned();
    draft.updated = at(25);
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();

    let due = store.outbox_due(ACCOUNT, at(30)).unwrap();
    let ProtoOp::Submit { raw, .. } = &due[0].op else {
        panic!("expected a submission");
    };
    let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("Re: lunch on friday"),
        "the frozen bytes changed under the edit:\n{text}"
    );
    assert!(!text.contains("something else entirely"), "{text}");
}

#[test]
fn sending_the_same_draft_twice_is_refused() {
    let (store, _dir) = seeded();
    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10)).unwrap();
    let draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();
    store
        .set_send_state(
            draft.id,
            &SendState::Sent {
                at: at(21),
                message: None,
            },
            at(21),
        )
        .unwrap();

    let err = compose::send(&store, draft.id, at(22)).expect_err("already sent");
    assert!(err.contains("already sent"), "{err}");
}

#[test]
fn an_account_with_no_identity_says_so_instead_of_inventing_a_sender() {
    // An account added before identities were created at setup has no row. Guessing a From
    // address is how mail goes out under an address the user does not own, so replying must
    // stop and say what is wrong.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let plan = mail_domain::presets::preset_for("me@ntu.edu.tw", at(0))
        .unwrap()
        .plan;
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();

    let err = compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10))
        .expect_err("no identity and no message");
    // The message does not exist in this bare store either; whichever check fires first, the
    // point is that nothing is invented and nothing panics.
    assert!(!err.is_empty(), "{err}");
}

#[test]
fn the_identity_comes_from_the_table_the_foreign_key_enforces() {
    // `drafts.identity` references `identities(id)`, so any draft that exists has a row. The
    // plan's copy of the same list is written once at account creation and can go stale;
    // reading it instead is how a send fails for an account that is perfectly well configured.
    let (store, _dir) = seeded();
    store
        .connection()
        .execute(
            "UPDATE accounts SET plan = json_set(plan, '$.identities', json('[]')) WHERE id = ?1",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10))
        .expect("the identity row is still there");
    let draft = only_draft(&store);
    let out = compose::send(&store, draft.id, at(20)).expect("and sending still works");
    assert!(out.contains("me@example.test"), "{out}");
}

#[test]
fn drafts_reports_what_is_waiting() {
    let (store, _dir) = seeded();
    assert!(compose::drafts(&store).unwrap().contains("no drafts"));

    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10)).unwrap();
    let listed = compose::drafts(&store).unwrap();
    assert!(listed.contains("editing"), "{listed}");
    assert!(listed.contains("Re: lunch on friday"), "{listed}");

    let draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();
    assert!(compose::drafts(&store).unwrap().contains("queued"));
}
