//! Drafts survive a round trip, in both [`Store`] implementations.
//!
//! This file exists because they did not. `MemoryStore` kept drafts in a map while the SQLite
//! `write_change` matched `Change::DraftUpsert(_) => None` and dropped them on the floor — a
//! composed reply was accepted, reported as applied, and gone on the next read. The parity
//! proptest could not see it: with no way to *read* a draft back, the two stores agreed on
//! every question anyone could ask them.
//!
//! So these tests ask the question directly, of both, and compare.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

/// Both stores, with the account and identity rows the foreign keys require.
fn both() -> Both {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = sqlite.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            [IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    Both {
        sqlite,
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

fn addr(email: &str) -> Address {
    Address {
        name: None,
        email: email.to_owned(),
    }
}

/// A draft with every optional field populated, so the round trip proves each column.
fn full_draft(in_reply_to: Option<MessageId>) -> Draft {
    Draft {
        id: DraftId::generate(),
        account: ACCOUNT,
        identity: IDENTITY,
        to: vec![addr("one@example.test"), addr("two@example.test")],
        cc: vec![addr("carbon@example.test")],
        bcc: vec![addr("blind@example.test")],
        subject: "Re: the thing".to_owned(),
        in_reply_to,
        forward_of: None,
        text: "quoted\n> original\n".to_owned(),
        html: Some("<p>quoted</p>".to_owned()),
        attachments: vec![],
        receipt: ReceiptRequest::Requested,
        state: SendState::Editing,
        updated: at(10),
    }
}

fn upsert(b: &Both, draft: &Draft) {
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
    };
    b.sqlite.apply(ACCOUNT, &patch).unwrap();
    b.memory.apply(ACCOUNT, &patch).unwrap();
}

#[test]
fn a_saved_draft_can_be_read_back_from_both_stores() {
    let b = both();
    let draft = full_draft(None);
    upsert(&b, &draft);

    // The regression: this was `Err(NoDraft)` from SQLite and `Ok` from memory.
    assert_eq!(b.sqlite.draft(draft.id).unwrap(), draft);
    assert_eq!(b.memory.draft(draft.id).unwrap(), draft);
}

#[test]
fn every_field_survives_sqlite_unchanged() {
    // Spelled out rather than left to the `assert_eq!` above, because a `Vec<Address>` that
    // round-trips as `[]` would still compare equal to a draft that was *built* with none —
    // and the recipients column is exactly the one a JSON shape change would silently empty.
    let b = both();
    let draft = full_draft(None);
    upsert(&b, &draft);
    let read = b.sqlite.draft(draft.id).unwrap();

    assert_eq!(read.to.len(), 2, "both recipients");
    assert_eq!(read.cc, vec![addr("carbon@example.test")]);
    assert_eq!(
        read.bcc,
        vec![addr("blind@example.test")],
        "bcc is not lost"
    );
    assert_eq!(read.html.as_deref(), Some("<p>quoted</p>"));
    assert_eq!(read.text, "quoted\n> original\n");
    assert_eq!(read.identity, IDENTITY);
    assert_eq!(read.updated, at(10));
}

#[test]
fn a_reply_keeps_the_message_it_answers() {
    // `in_reply_to` is a real foreign key into `messages`, so this also proves the draft is
    // written *after* the message exists rather than being quietly rejected.
    let b = both();
    let message = ingest_one(&b);
    let draft = full_draft(Some(message));
    upsert(&b, &draft);

    assert_eq!(b.sqlite.draft(draft.id).unwrap().in_reply_to, Some(message));
    assert_eq!(b.memory.draft(draft.id).unwrap().in_reply_to, Some(message));
}

#[test]
fn drafts_list_newest_first_in_both_stores() {
    let b = both();
    let mut older = full_draft(None);
    older.updated = at(1);
    let mut newer = full_draft(None);
    newer.updated = at(99);
    upsert(&b, &older);
    upsert(&b, &newer);

    let sqlite: Vec<DraftId> = b
        .sqlite
        .drafts(ACCOUNT)
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    let memory: Vec<DraftId> = b
        .memory
        .drafts(ACCOUNT)
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(sqlite, vec![newer.id, older.id]);
    assert_eq!(sqlite, memory, "the two stores must agree on order");
}

#[test]
fn saving_the_same_draft_again_updates_rather_than_duplicates() {
    // A composer autosaves on a timer. If each save inserted a row, the drafts list would grow
    // one entry per keystroke.
    let b = both();
    let mut draft = full_draft(None);
    upsert(&b, &draft);
    draft.subject = "edited".to_owned();
    draft.updated = at(20);
    upsert(&b, &draft);

    assert_eq!(b.sqlite.drafts(ACCOUNT).unwrap().len(), 1);
    assert_eq!(b.memory.drafts(ACCOUNT).unwrap().len(), 1);
    assert_eq!(b.sqlite.draft(draft.id).unwrap().subject, "edited");
    assert_eq!(b.memory.draft(draft.id).unwrap().subject, "edited");
}

#[test]
fn deleting_a_draft_removes_it_from_both() {
    let b = both();
    let draft = full_draft(None);
    upsert(&b, &draft);
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::DraftDelete(draft.id)],
    };
    b.sqlite.apply(ACCOUNT, &patch).unwrap();
    b.memory.apply(ACCOUNT, &patch).unwrap();

    assert!(matches!(
        b.sqlite.draft(draft.id),
        Err(StoreError::NoDraft(_))
    ));
    assert!(matches!(
        b.memory.draft(draft.id),
        Err(StoreError::NoDraft(_))
    ));
}

#[test]
fn the_send_state_advances_without_touching_the_rest() {
    let b = both();
    let draft = full_draft(None);
    upsert(&b, &draft);

    let sent = SendState::Sent {
        at: at(50),
        message: None,
    };
    b.sqlite.set_send_state(draft.id, &sent, at(50)).unwrap();
    b.memory.set_send_state(draft.id, &sent, at(50)).unwrap();

    for read in [
        b.sqlite.draft(draft.id).unwrap(),
        b.memory.draft(draft.id).unwrap(),
    ] {
        assert_eq!(read.state, sent);
        assert_eq!(read.updated, at(50), "the transition is a touch");
        assert_eq!(read.subject, draft.subject, "the body is untouched");
        assert_eq!(read.to, draft.to);
    }
}

#[test]
fn advancing_a_draft_that_is_gone_is_an_error_not_a_silent_insert() {
    // `UPDATE ... WHERE id = ?` matches nothing and reports success. If the send path took that
    // as "sent", a draft the user deleted mid-flight would be marked delivered.
    let b = both();
    let ghost = DraftId::generate();
    let state = SendState::Queued;
    assert!(matches!(
        b.sqlite.set_send_state(ghost, &state, at(1)),
        Err(StoreError::NoDraft(_))
    ));
    assert!(matches!(
        b.memory.set_send_state(ghost, &state, at(1)),
        Err(StoreError::NoDraft(_))
    ));
}

/// Ingest one message and return its id, so a draft can legally reference it.
fn ingest_one(b: &Both) -> MessageId {
    let thread = ThreadId::generate();
    let raw = b
        .sqlite
        .blobs()
        .put(&b.sqlite.connection(), b"raw bytes")
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("original@example.test".to_owned()),
        date: at(0),
        from: addr("sender@example.test"),
        reply_to: vec![],
        to: vec![addr("me@example.test")],
        cc: vec![],
        bcc: vec![],
        subject: "the thing".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("original@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("hello".to_owned()),
            raw,
        },
        attachments: vec![],
    };
    let id = message.id;
    let ingest = Ingest {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        validity: UidValidity::Same,
        cursor: Some(SyncCursor::Pop),
        messages: vec![Fetched {
            remote: RemoteRef::Pop {
                uidl: "u1".to_owned(),
            },
            key: message.key.clone(),
            raw,
            message: message.clone(),
        }],
        flags: vec![],
        labels: vec![],
        label_names: Vec::new(),
        gone: vec![],
    };
    b.sqlite.ingest(ACCOUNT, ingest.clone()).unwrap();
    b.memory.ingest(ACCOUNT, ingest).unwrap();
    id
}

// ---------------------------------------------------------------------------------------
// Read receipts: the draft that asks, and the answer to a message that asked.
// ---------------------------------------------------------------------------------------

#[test]
fn a_draft_asking_for_a_receipt_still_asks_when_read_back() {
    let b = both();
    let mut draft = full_draft(None);
    draft.receipt = ReceiptRequest::Requested;
    upsert(&b, &draft);
    assert_eq!(
        b.sqlite.draft(draft.id).unwrap().receipt,
        ReceiptRequest::Requested
    );
    assert_eq!(
        b.memory.draft(draft.id).unwrap().receipt,
        ReceiptRequest::Requested
    );
}

#[test]
fn a_receipt_answer_is_kept_and_the_first_one_stands_in_both_stores() {
    let b = both();
    let message = ingest_one(&b);
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert_eq!(store.receipt_answer(message).unwrap(), None);
        assert_eq!(
            store
                .answer_receipt(message, ReceiptAnswer::Declined, at(1))
                .unwrap(),
            ReceiptAnswer::Declined
        );
        // Asked once: a later answer does not replace the first.
        assert_eq!(
            store
                .answer_receipt(message, ReceiptAnswer::Sent, at(2))
                .unwrap(),
            ReceiptAnswer::Declined
        );
        assert_eq!(
            store.receipt_answer(message).unwrap(),
            Some(ReceiptAnswer::Declined)
        );
    }
}

#[test]
fn answering_for_a_message_that_is_not_there_is_an_error_in_both_stores() {
    let b = both();
    let ghost = MessageId::generate();
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert!(matches!(
            store.answer_receipt(ghost, ReceiptAnswer::Sent, at(1)),
            Err(StoreError::NoMessage(id)) if id == ghost
        ));
        assert_eq!(store.receipt_answer(ghost).unwrap(), None);
    }
}

#[test]
fn a_keyword_intent_resolves_to_the_same_operation_in_both_stores() {
    let b = both();
    let message = ingest_one(&b);
    let intent = RemoteIntent::AddKeyword {
        messages: vec![message],
        keyword: Keyword::MdnSent,
    };
    let nothing = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };
    let mut ops = Vec::new();
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert!(
            store
                .enqueue(ACCOUNT, intent.clone(), &nothing, at(1))
                .unwrap()
                .is_some()
        );
        let due = store.outbox_due(ACCOUNT, at(1_000)).unwrap();
        assert_eq!(due.len(), 1);
        ops.push(due[0].op.clone());
    }
    assert_eq!(ops[0], ops[1]);
    assert_eq!(
        ops[0],
        ProtoOp::AddKeyword {
            remotes: vec![RemoteRef::Pop {
                uidl: "u1".to_owned()
            }],
            keyword: Keyword::MdnSent,
        }
    );
}
