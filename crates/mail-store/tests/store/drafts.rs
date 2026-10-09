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
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};
use porter_core::AccountId;

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
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
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            [IDENTITY.to_string(), acct_account().to_string()],
        )
        .unwrap();
    }
    Both {
        sqlite,
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

impl Both {
    /// Each store, named for a failure message.
    fn each(&self) -> [(&'static str, &dyn Store); 2] {
        [("sqlite", &self.sqlite), ("memory", &self.memory)]
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
        account: acct_account(),
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
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: at(10),
    }
}

fn upsert(b: &Both, draft: &Draft) {
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
    };
    b.sqlite.apply(acct_account(), &patch).unwrap();
    b.memory.apply(acct_account(), &patch).unwrap();
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

/// Drafts list newest first, saving one again updates it in place, and deleting one removes it:
/// each step asked of both stores.
#[test]
fn drafts_in_both_stores_list_newest_first_resave_in_place_and_delete() {
    let b = both();
    let mut older = full_draft(None);
    older.updated = at(1);
    let mut newer = full_draft(None);
    newer.updated = at(99);
    upsert(&b, &older);
    upsert(&b, &newer);

    fn ids(store: &dyn Store) -> Vec<DraftId> {
        store
            .drafts(acct_account())
            .unwrap()
            .iter()
            .map(|d| d.id)
            .collect()
    }
    assert_eq!(
        ids(&b.sqlite),
        vec![newer.id, older.id],
        "listing: newest first"
    );
    assert_eq!(
        ids(&b.sqlite),
        ids(&b.memory),
        "listing: the two stores must agree on order"
    );

    // A composer autosaves on a timer. If each save inserted a row, the drafts list would grow
    // one entry per keystroke.
    newer.subject = "edited".to_owned();
    newer.updated = at(100);
    upsert(&b, &newer);
    for (label, store) in b.each() {
        assert_eq!(
            ids(store),
            vec![newer.id, older.id],
            "{label}: saving again updates rather than duplicates"
        );
        assert_eq!(
            store.draft(newer.id).unwrap().subject,
            "edited",
            "{label}: resave"
        );
    }

    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::DraftDelete(newer.id)],
    };
    for (label, store) in b.each() {
        store.apply(acct_account(), &patch).unwrap();
        assert!(
            matches!(store.draft(newer.id), Err(StoreError::NoDraft(_))),
            "{label}: a deleted draft is gone"
        );
        assert_eq!(ids(store), vec![older.id], "{label}: the other stays");
    }
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
        .put(b"raw bytes")
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account: acct_account(),
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
            account: acct_account(),
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
    b.sqlite.ingest(acct_account(), ingest.clone()).unwrap();
    b.memory.ingest(acct_account(), ingest).unwrap();
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

// ---------------------------------------------------------------------------------------
// Calendar invitations: the answer the user last gave one.
// ---------------------------------------------------------------------------------------

fn invite_answer(message: MessageId, attendance: Attendance, n: i64) -> InviteAnswer {
    InviteAnswer {
        message,
        attendance,
        sequence: 3,
        comment: Some(format!("note {n}")),
        answered_at: at(n),
    }
}

#[test]
fn an_invitation_answer_is_kept_and_a_later_one_replaces_it_in_both_stores() {
    let b = both();
    let message = ingest_one(&b);
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert_eq!(store.invite_answer(message).unwrap(), None);
        let first = invite_answer(message, Attendance::Tentative, 1);
        store.answer_invite(&first).unwrap();
        assert_eq!(store.invite_answer(message).unwrap(), Some(first));
        // A person may change their mind about a meeting; the answer that stands is the last.
        let second = InviteAnswer {
            comment: None,
            ..invite_answer(message, Attendance::Declined, 2)
        };
        store.answer_invite(&second).unwrap();
        assert_eq!(store.invite_answer(message).unwrap(), Some(second));
    }
}

#[test]
fn answering_an_invitation_that_is_not_there_is_an_error_in_both_stores() {
    let b = both();
    let ghost = MessageId::generate();
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert!(matches!(
            store.answer_invite(&invite_answer(ghost, Attendance::Accepted, 1)),
            Err(StoreError::NoMessage(id)) if id == ghost
        ));
        assert_eq!(store.invite_answer(ghost).unwrap(), None);
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
                .enqueue(acct_account(), intent.clone(), &nothing, at(1))
                .unwrap()
                .is_some()
        );
        let due = store.outbox_due(acct_account(), at(1_000)).unwrap();
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
