//! A JMAP account in the store: addresses under the one mailbox `JMAP_ALL`, and filing told
//! outright by the server (`Store::refile`).
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares their
//! answers, the parity the rest of the store is held to.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn both<T>(scenario: impl Fn(&dyn Store) -> T) -> (T, T) {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    sqlite
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    let memory = MemoryStore::new();
    (scenario(&sqlite), scenario(&memory))
}

fn all() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: JMAP_ALL.to_owned(),
    }
}

fn jmap(id: &str) -> RemoteRef {
    RemoteRef::Jmap {
        email_id: id.to_owned(),
    }
}

fn message(n: u128, role: MailboxRole) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000)),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: role,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: all(),
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Jmap {
                    email_state: "e1".to_owned(),
                    mailbox_state: "m1".to_owned(),
                }),
                messages: vec![Fetched {
                    remote,
                    key: m.key.clone(),
                    raw: BlobId::generate(),
                    message: m.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
}

fn role(store: &dyn Store, n: u128) -> MailboxRole {
    store
        .message(MessageId::from_uuid(uuid::Uuid::from_u128(n)))
        .unwrap()
        .mailbox
}

#[test]
fn a_jmap_address_reads_back_as_one_and_the_cursor_with_it() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), jmap("M1"));
        (
            store.remote_refs(&all()).unwrap(),
            store
                .remotes_of(MessageId::from_uuid(uuid::Uuid::from_u128(1)))
                .unwrap(),
            store.unfetched_in(&all(), 10).unwrap(),
            store.cursor(&all()).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let (listed, of_message, unfetched, cursor) = sqlite;
    assert_eq!(listed, vec![jmap("M1")]);
    assert_eq!(of_message, vec![jmap("M1")]);
    assert_eq!(unfetched, vec![jmap("M1")]);
    assert_eq!(
        cursor,
        Some(SyncCursor::Jmap {
            email_state: "e1".to_owned(),
            mailbox_state: "m1".to_owned()
        })
    );
}

#[test]
fn the_server_refiles_a_message_moved_elsewhere() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), jmap("M1"));
        let before = role(store, 1);
        let moved = store
            .refile(ACCOUNT, &[(jmap("M1"), MailboxRole::Archive)])
            .unwrap();
        let after = role(store, 1);
        // Already filed so, and an address nobody holds: nothing to change.
        let again = store
            .refile(
                ACCOUNT,
                &[
                    (jmap("M1"), MailboxRole::Archive),
                    (jmap("unknown"), MailboxRole::Trash),
                ],
            )
            .unwrap();
        (before, moved.changes, after, again.changes)
    });
    assert_eq!(sqlite, memory);
    let (before, moved, after, again) = sqlite;
    let id = MessageId::from_uuid(uuid::Uuid::from_u128(1));
    assert_eq!(before, MailboxRole::Inbox);
    assert_eq!(
        moved,
        vec![Change::MessageMailbox(id, MailboxRole::Archive)]
    );
    assert_eq!(after, MailboxRole::Archive);
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn a_local_move_the_server_has_not_seen_survives_refiling() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), jmap("M1"));
        let id = MessageId::from_uuid(uuid::Uuid::from_u128(1));
        // Trashed here and queued; the server still says Inbox until the outbox drains.
        let forward = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::MessageMailbox(id, MailboxRole::Trash)],
        };
        let undo = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::MessageMailbox(id, MailboxRole::Inbox)],
        };
        store.apply(ACCOUNT, &forward).unwrap();
        store
            .enqueue(
                ACCOUNT,
                RemoteIntent::SetMailbox {
                    messages: vec![id],
                    role: MailboxRole::Trash,
                },
                &undo,
                at(1),
            )
            .unwrap();
        store
            .refile(ACCOUNT, &[(jmap("M1"), MailboxRole::Archive)])
            .unwrap();
        let queued: Vec<ProtoOp> = store
            .outbox_due(ACCOUNT, at(10))
            .unwrap()
            .into_iter()
            .map(|e| e.op)
            .collect();
        (role(store, 1), queued)
    });
    assert_eq!(sqlite, memory);
    let (after, queued) = sqlite;
    assert_eq!(after, MailboxRole::Trash);
    // And the queued move addresses the email by its JMAP id.
    assert_eq!(
        queued,
        vec![ProtoOp::SetMailbox {
            remotes: vec![jmap("M1")],
            role: MailboxRole::Trash
        }]
    );
}

#[test]
fn an_email_gone_from_the_account_is_gone_from_here() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), jmap("M1"));
        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: all(),
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![],
                    gone: vec![jmap("M1")],
                },
            )
            .unwrap();
        (
            store
                .message(MessageId::from_uuid(uuid::Uuid::from_u128(1)))
                .is_err(),
            store.remote_refs(&all()).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, (true, vec![]));
}
