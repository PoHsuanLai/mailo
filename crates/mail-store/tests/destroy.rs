//! Delete forever: the message goes here at once, and its server addresses are kept until the
//! server no longer lists them.
//!
//! Without them the next sync would find a UID with no row and fetch the message back into
//! Trash, and the queued deletion would find its message gone and send nothing. Every scenario
//! runs against the SQLite store and the in-memory one and compares what each answers.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{Dispatch, MemoryStore, Settle, SqliteStore, Store};

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

fn imap(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 7,
        uid,
    }
}

fn mailbox(path: &str) -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(vec![("Trash".to_owned(), MailboxRole::Trash)]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 2 },
        observed_at: at(0),
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

/// `m` arrives at `remote`.
fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    let RemoteRef::Imap { mailbox, .. } = &remote else {
        panic!("IMAP only here");
    };
    let mut batch = ingest(mailbox);
    batch.messages.push(Fetched {
        remote,
        key: m.key.clone(),
        raw: BlobId::generate(),
        message: m.clone(),
    });
    store.ingest(ACCOUNT, batch).unwrap();
}

/// What the window does for "Delete forever": the op applied to the thread, the deletion queued
/// while the store still knows where the message is, then the message removed here.
fn delete_forever(store: &dyn Store, m: &Message) -> Option<OutboxId> {
    let thread = store.thread(m.thread).unwrap();
    let messages: Vec<Message> = thread
        .messages
        .iter()
        .map(|id| store.message(*id).unwrap())
        .collect();
    let applied = Op::Destroy.apply(
        &Target::Threads(vec![m.thread]),
        &thread,
        &messages,
        &caps(),
        at(1),
    );
    assert!(applied.inverse.changes.is_empty(), "nothing to undo");
    let queued = applied.remote.and_then(|intent| {
        store
            .enqueue(ACCOUNT, intent, &applied.inverse, at(1))
            .unwrap()
    });
    store.apply(ACCOUNT, &applied.forward).unwrap();
    queued
}

fn held(store: &dyn Store, m: &Message) -> bool {
    store.message(m.id).is_ok()
}

#[test]
fn a_destroyed_message_goes_here_and_its_address_stays_counted_as_held() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Trash);
        deliver(store, &m, imap("Trash", 10));
        let before = store.remote_refs(&mailbox("Trash")).unwrap();

        let entry = delete_forever(store, &m).expect("queued");
        (
            before,
            held(store, &m),
            store.remote_refs(&mailbox("Trash")).unwrap(),
            store.outbox_dispatch(entry).unwrap(),
            store.outbox_due(ACCOUNT, at(2)).unwrap()[0].op.clone(),
        )
    });
    assert_eq!(sqlite, memory);
    let (before, still_held, refs, dispatch, listed) = sqlite;
    assert_eq!(before, vec![imap("Trash", 10)]);
    assert!(!still_held, "gone here at once");
    // A sync must not see UID 10 as new mail and fetch it back.
    assert_eq!(refs, vec![imap("Trash", 10)]);
    assert_eq!(
        dispatch,
        Dispatch::Send(ProtoOp::Destroy {
            remotes: vec![imap("Trash", 10)]
        })
    );
    assert_eq!(Dispatch::Send(listed), dispatch);
}

#[test]
fn a_confirmed_deletion_keeps_the_address_until_a_sync_no_longer_finds_it() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Trash);
        deliver(store, &m, imap("Trash", 10));
        let entry = delete_forever(store, &m).expect("queued");
        store.outbox_settle(entry, Settle::Ok, at(2)).unwrap();
        let settled = store.remote_refs(&mailbox("Trash")).unwrap();
        let mut sweep = ingest("Trash");
        sweep.gone = vec![imap("Trash", 10)];
        store.ingest(ACCOUNT, sweep).unwrap();
        (
            settled,
            store.remote_refs(&mailbox("Trash")).unwrap(),
            store.outbox_due(ACCOUNT, at(3)).unwrap().len(),
        )
    });
    assert_eq!(sqlite, memory);
    let (settled, swept, queued) = sqlite;
    // POP3 answers at once and the server keeps the message: the UIDL must stay held.
    assert_eq!(settled, vec![imap("Trash", 10)]);
    assert_eq!(swept, Vec::<RemoteRef>::new());
    assert_eq!(queued, 0);
}

#[test]
fn a_refused_deletion_lets_the_next_sync_bring_the_message_back() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Trash);
        deliver(store, &m, imap("Trash", 10));
        let entry = delete_forever(store, &m).expect("queued");
        store
            .outbox_settle(
                entry,
                Settle::Failed {
                    reason: "the server does not offer UIDPLUS".to_owned(),
                    retry: Retry::Fatal("no UIDPLUS".to_owned()),
                },
                at(2),
            )
            .unwrap();
        let refs = store.remote_refs(&mailbox("Trash")).unwrap();
        // The next sync fetches what it does not hold, and the message is back as it is there.
        deliver(store, &m, imap("Trash", 10));
        (refs, held(store, &m))
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite.0, Vec::<RemoteRef>::new());
    assert!(sqlite.1);
}

#[test]
fn a_retry_keeps_the_address_and_sends_to_it_again() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Trash);
        deliver(store, &m, imap("Trash", 10));
        let entry = delete_forever(store, &m).expect("queued");
        store
            .outbox_settle(
                entry,
                Settle::Failed {
                    reason: "connection reset".to_owned(),
                    retry: Retry::Now,
                },
                at(2),
            )
            .unwrap();
        (
            store.remote_refs(&mailbox("Trash")).unwrap(),
            store.outbox_dispatch(entry).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite.0, vec![imap("Trash", 10)]);
    assert_eq!(
        sqlite.1,
        Dispatch::Send(ProtoOp::Destroy {
            remotes: vec![imap("Trash", 10)]
        })
    );
}

#[test]
fn a_move_to_trash_still_queued_takes_the_deletion_with_it() {
    // Trashed and then deleted forever before the move reached the server: the move is still
    // sent, from where the message was, and the deletion follows it to where the server said it
    // landed (COPYUID), not to the inbox UID it was queued with.
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Inbox);
        deliver(store, &m, imap("INBOX", 10));
        let thread = store.thread(m.thread).unwrap();
        let trashed = Op::Trash.apply(
            &Target::Threads(vec![m.thread]),
            &thread,
            std::slice::from_ref(&m),
            &caps(),
            at(1),
        );
        let first = store
            .enqueue(
                ACCOUNT,
                trashed.remote.clone().expect("a move"),
                &trashed.inverse,
                at(1),
            )
            .unwrap()
            .expect("queued");
        store.apply(ACCOUNT, &trashed.forward).unwrap();
        let second = delete_forever(store, &m).expect("queued");

        let move_sent = store.outbox_dispatch(first).unwrap();
        store
            .remap(ACCOUNT, &imap("INBOX", 10), &imap("Trash", 55))
            .unwrap();
        store.outbox_settle(first, Settle::Ok, at(2)).unwrap();
        (
            move_sent,
            store.outbox_dispatch(second).unwrap(),
            store.remote_refs(&mailbox("Trash")).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(
        sqlite.0,
        Dispatch::Send(ProtoOp::SetMailbox {
            remotes: vec![imap("INBOX", 10)],
            role: MailboxRole::Trash,
        }),
        "the move is not dropped for its message having gone here"
    );
    assert_eq!(
        sqlite.1,
        Dispatch::Send(ProtoOp::Destroy {
            remotes: vec![imap("Trash", 55)]
        })
    );
    assert_eq!(sqlite.2, vec![imap("Trash", 55)]);
}

#[test]
fn outside_trash_and_spam_nothing_is_removed_or_queued() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Inbox);
        deliver(store, &m, imap("INBOX", 10));
        (
            delete_forever(store, &m),
            held(store, &m),
            store.outbox_due(ACCOUNT, at(2)).unwrap().len(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, (None, true, 0));
}

#[test]
fn a_renumbered_or_deleted_trash_forgets_what_it_kept() {
    let (sqlite, memory) = both(|store| {
        let m = message(1, MailboxRole::Trash);
        deliver(store, &m, imap("Trash", 10));
        let n = message(2, MailboxRole::Spam);
        deliver(store, &n, imap("Junk", 11));
        delete_forever(store, &m).expect("queued");
        delete_forever(store, &n).expect("queued");
        let mut reset = ingest("Trash");
        reset.validity = UidValidity::Reset;
        store.ingest(ACCOUNT, reset).unwrap();
        (
            store.remote_refs(&mailbox("Trash")).unwrap(),
            store.remote_refs(&mailbox("Junk")).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite.0, Vec::<RemoteRef>::new());
    assert_eq!(sqlite.1, vec![imap("Junk", 11)]);
}
