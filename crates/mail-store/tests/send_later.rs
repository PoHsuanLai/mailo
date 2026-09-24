//! A send held in the outbox until a time: not handed out early, handed out when due, found by
//! a watch deciding how long to sleep, and still there after a restart. Both stores, compared.
//!
//! The hold is the outbox entry's `next_attempt`, which retries have always used. Nothing new
//! decides when a scheduled send goes; these tests pin that the old rule answers the new
//! question.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, Settle, SqliteStore, Store};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

/// When the scheduled send may leave.
const LEAVES: i64 = 3_600;

fn seed(sqlite: &SqliteStore) {
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

fn draft() -> Draft {
    Draft {
        id: DraftId::generate(),
        account: ACCOUNT,
        identity: IDENTITY,
        to: vec![Address {
            name: None,
            email: "you@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "tomorrow morning".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "hello".to_owned(),
        html: None,
        attachments: vec![],
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        state: SendState::Editing,
        updated: at(0),
    }
}

/// Save `draft`, and queue its submission to leave at `leaves`, as `compose::queue` does.
fn schedule(store: &dyn Store, draft: &Draft, leaves: DateTime<Utc>) -> OutboxId {
    let empty = Patch {
        id: ChangeId::generate(),
        changes: Vec::new(),
    };
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let id = store
        .enqueue(
            ACCOUNT,
            RemoteIntent::Send {
                draft: draft.id,
                raw: BlobId::generate(),
                mail_from: "me@example.test".to_owned(),
                rcpt_to: vec!["you@example.test".to_owned()],
            },
            &empty,
            leaves,
        )
        .unwrap()
        .expect("a submission is always queued");
    store
        .set_send_state(draft.id, &SendState::Scheduled { at: leaves }, at(0))
        .unwrap();
    id
}

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

fn both() -> Both {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    seed(&sqlite);
    Both {
        sqlite,
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

impl Both {
    fn each(&self) -> [(&'static str, &dyn Store); 2] {
        [("sqlite", &self.sqlite), ("memory", &self.memory)]
    }
}

#[test]
fn a_scheduled_send_is_not_handed_out_before_its_time() {
    let b = both();
    let draft = draft();
    for (name, store) in b.each() {
        schedule(store, &draft, at(LEAVES));
        for early in [0, 1, LEAVES - 1] {
            assert_eq!(
                store.outbox_due(ACCOUNT, at(early)).unwrap(),
                vec![],
                "{name}: handed out {} seconds early",
                LEAVES - early
            );
        }
    }
}

#[test]
fn and_is_handed_out_from_its_time_on() {
    let b = both();
    let draft = draft();
    for (name, store) in b.each() {
        let id = schedule(store, &draft, at(LEAVES));
        for late in [LEAVES, LEAVES + 1, LEAVES + 86_400] {
            let due = store.outbox_due(ACCOUNT, at(late)).unwrap();
            assert_eq!(
                due.iter().map(|e| e.id).collect::<Vec<_>>(),
                vec![id],
                "{name}: not due {} seconds after its time",
                late - LEAVES
            );
        }
    }
}

#[test]
fn the_draft_says_it_is_scheduled_and_when() {
    let b = both();
    let draft = draft();
    for (name, store) in b.each() {
        schedule(store, &draft, at(LEAVES));
        assert_eq!(
            store.draft(draft.id).unwrap().state,
            SendState::Scheduled { at: at(LEAVES) },
            "{name}"
        );
    }
}

#[test]
fn a_watch_learns_when_to_wake_from_the_outbox() {
    let b = both();
    let (soon, later) = (draft(), draft());
    for (name, store) in b.each() {
        assert_eq!(store.outbox_next(ACCOUNT, at(0)).unwrap(), None, "{name}");
        schedule(store, &later, at(LEAVES * 2));
        schedule(store, &soon, at(LEAVES));
        assert_eq!(
            store.outbox_next(ACCOUNT, at(0)).unwrap(),
            Some(at(LEAVES)),
            "{name}: the earliest, not the first queued"
        );
        // Strictly after: an entry already due is the pass's business, not the alarm's.
        assert_eq!(
            store.outbox_next(ACCOUNT, at(LEAVES)).unwrap(),
            Some(at(LEAVES * 2)),
            "{name}"
        );
        assert_eq!(
            store.outbox_next(ACCOUNT, at(LEAVES * 2)).unwrap(),
            None,
            "{name}"
        );
    }
}

#[test]
fn a_send_being_retried_does_not_wake_a_watch() {
    // Its backoff is its own schedule, and waking a whole pass for a one-second one would
    // hammer the server the backoff exists to spare.
    let b = both();
    let draft = draft();
    for (name, store) in b.each() {
        let id = schedule(store, &draft, at(LEAVES));
        store
            .outbox_settle(
                id,
                Settle::Failed {
                    reason: "connection refused".to_owned(),
                    retry: Retry::Now,
                },
                at(LEAVES),
            )
            .unwrap();
        assert_eq!(
            store.outbox_next(ACCOUNT, at(LEAVES)).unwrap(),
            None,
            "{name}"
        );
    }
}

#[test]
fn taking_the_draft_back_takes_the_send_with_it() {
    // What `unsend` writes: the draft deleted and put back, which withdraws its submission.
    let b = both();
    let draft = draft();
    for (name, store) in b.each() {
        schedule(store, &draft, at(LEAVES));
        let back = Draft {
            state: SendState::Editing,
            ..draft.clone()
        };
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![
                        Change::DraftDelete(draft.id),
                        Change::DraftUpsert(Box::new(back.clone())),
                    ],
                },
            )
            .unwrap();
        assert_eq!(
            store.outbox_due(ACCOUNT, at(LEAVES * 10)).unwrap(),
            vec![],
            "{name}"
        );
        assert_eq!(store.outbox_next(ACCOUNT, at(0)).unwrap(), None, "{name}");
        assert_eq!(store.draft(draft.id).unwrap(), back, "{name}");
    }
}

#[test]
fn a_scheduled_send_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let draft = draft();
    let id = {
        let store = SqliteStore::open(&path, dir.path()).unwrap();
        seed(&store);
        schedule(&store, &draft, at(LEAVES))
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(
        store.draft(draft.id).unwrap().state,
        SendState::Scheduled { at: at(LEAVES) }
    );
    assert_eq!(store.outbox_due(ACCOUNT, at(LEAVES - 1)).unwrap(), vec![]);
    assert_eq!(store.outbox_next(ACCOUNT, at(0)).unwrap(), Some(at(LEAVES)));
    assert_eq!(
        store
            .outbox_due(ACCOUNT, at(LEAVES))
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        vec![id]
    );
}
