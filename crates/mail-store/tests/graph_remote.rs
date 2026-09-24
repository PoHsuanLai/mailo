//! Microsoft Graph's addresses in `remote_map` (`plan.md` 10.18): kept and read back as Graph's,
//! never as POP3's, and moved to the id a folder move returns.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, the parity the rest of the store is held to.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

/// Run `scenario` on both stores and hand back what each produced.
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

fn mailbox(path: &str) -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    }
}

fn graph(mailbox: &str, id: &str) -> RemoteRef {
    RemoteRef::Graph {
        mailbox: mailbox.to_owned(),
        id: id.to_owned(),
    }
}

fn message(n: u128) -> Message {
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
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn ingest(path: &str) -> Ingest {
    Ingest {
        mailbox: mailbox(path),
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![],
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    }
}

fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    let mut batch = ingest("INBOX");
    batch.messages.push(Fetched {
        remote,
        key: m.key.clone(),
        raw: BlobId::generate(),
        message: m.clone(),
    });
    store.ingest(ACCOUNT, batch).unwrap();
}

#[test]
fn a_graph_address_reads_back_as_graph_in_every_query() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "AAMk/one=="));
        (
            store.remote_refs(&mailbox("INBOX")).unwrap(),
            store.remotes_of(m.id).unwrap(),
            store.unfetched_in(&mailbox("INBOX"), 10).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let expected = vec![graph("INBOX", "AAMk/one==")];
    assert_eq!(sqlite.0, expected);
    assert_eq!(sqlite.1, expected);
    assert_eq!(sqlite.2, expected);
}

#[test]
fn a_move_remaps_the_message_to_its_new_id_and_folder() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "old"));
        store
            .remap(ACCOUNT, &graph("INBOX", "old"), &graph("Archive", "new"))
            .unwrap();
        // The folder it left says it has gone: that is not a deletion any more.
        let mut left = ingest("INBOX");
        left.gone.push(graph("INBOX", "old"));
        store.ingest(ACCOUNT, left).unwrap();
        // And the folder it arrived in reports it read, under the new id.
        let mut arrived = ingest("Archive");
        arrived
            .flags
            .push((graph("Archive", "new"), ReadState::Read, Star::Unstarred));
        store.ingest(ACCOUNT, arrived).unwrap();
        let held = store.message(m.id).unwrap();
        (
            store.remote_refs(&mailbox("INBOX")).unwrap(),
            store.remotes_of(m.id).unwrap(),
            held.read,
        )
    });
    assert_eq!(sqlite, memory);
    assert!(sqlite.0.is_empty(), "{:?}", sqlite.0);
    assert_eq!(sqlite.1, vec![graph("Archive", "new")]);
    assert_eq!(sqlite.2, ReadState::Read);
}

#[test]
fn a_remap_onto_an_address_a_sync_already_mapped_leaves_one_row() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "old"));
        let mut there = ingest("Archive");
        there.messages.push(Fetched {
            remote: graph("Archive", "new"),
            key: m.key.clone(),
            raw: BlobId::generate(),
            message: m.clone(),
        });
        store.ingest(ACCOUNT, there).unwrap();
        store
            .remap(ACCOUNT, &graph("INBOX", "old"), &graph("Archive", "new"))
            .unwrap();
        store.remotes_of(m.id).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, vec![graph("Archive", "new")]);
}

#[test]
fn remapping_an_address_nobody_holds_does_nothing() {
    let (sqlite, memory) = both(|store| {
        store
            .remap(ACCOUNT, &graph("INBOX", "never"), &graph("Archive", "x"))
            .unwrap();
        store.remote_refs(&mailbox("Archive")).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert!(sqlite.is_empty());
}
