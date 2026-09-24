//! A queued operation is addressed when it is sent, not when it was queued (FINDINGS F153).
//!
//! A second move queued behind a first was resolved to the address the first moved the message
//! away from, sent there, answered OK — a UID set naming nothing is not an error — and lost.
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, the parity the rest of the store is held to.

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
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: path.to_owned(),
        },
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![],
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    }
}

/// `m` arrives at `remote`, in the mailbox the address names.
fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    let path = match &remote {
        RemoteRef::Imap { mailbox, .. } | RemoteRef::Graph { mailbox, .. } => mailbox.clone(),
        RemoteRef::Pop { .. } | RemoteRef::Jmap { .. } => "INBOX".to_owned(),
    };
    let mut batch = ingest(&path);
    batch.messages.push(Fetched {
        remote,
        key: m.key.clone(),
        raw: BlobId::generate(),
        message: m.clone(),
    });
    store.ingest(ACCOUNT, batch).unwrap();
}

fn queue(store: &dyn Store, intent: RemoteIntent) -> OutboxId {
    store
        .enqueue(
            ACCOUNT,
            intent,
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            at(0),
        )
        .unwrap()
        .expect("queued")
}

fn file_into(m: &Message, role: MailboxRole) -> RemoteIntent {
    RemoteIntent::SetMailbox {
        messages: vec![m.id],
        role,
    }
}

/// The addresses an operation would be sent to.
fn remotes(dispatch: &Dispatch) -> Vec<RemoteRef> {
    match dispatch {
        Dispatch::Send(
            ProtoOp::SetMailbox { remotes, .. }
            | ProtoOp::SetFlags { remotes, .. }
            | ProtoOp::File { remotes, .. }
            | ProtoOp::SetLabels { remotes, .. }
            | ProtoOp::AddKeyword { remotes, .. },
        ) => remotes.clone(),
        other => panic!("not an operation on messages: {other:?}"),
    }
}

/// The first of two moves goes, and the server says where the message went.
fn first_move_lands(store: &dyn Store, first: OutboxId, to: Option<RemoteRef>) {
    let sent = store.outbox_dispatch(first).unwrap();
    assert_eq!(remotes(&sent), vec![imap("INBOX", 10)]);
    match to {
        Some(to) => store.remap(ACCOUNT, &imap("INBOX", 10), &to).unwrap(),
        None => store.unmap(ACCOUNT, &imap("INBOX", 10)).unwrap(),
    }
    store.outbox_settle(first, Settle::Ok, at(1)).unwrap();
}

#[test]
fn a_second_queued_move_is_sent_where_the_first_put_the_message() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let first = queue(store, file_into(&m, MailboxRole::Archive));
        let second = queue(store, file_into(&m, MailboxRole::Trash));
        // Both were queued with the one address the message had.
        let queued = store.outbox_due(ACCOUNT, at(0)).unwrap();
        assert_eq!(queued.len(), 2);

        first_move_lands(store, first, Some(imap("Archive", 77)));
        (
            store.outbox_dispatch(second).unwrap(),
            store.outbox_due(ACCOUNT, at(1)).unwrap()[0].op.clone(),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(
        sqlite.0,
        Dispatch::Send(ProtoOp::SetMailbox {
            remotes: vec![imap("Archive", 77)],
            role: MailboxRole::Trash,
        })
    );
    // What the queue lists is where it would go now, too.
    assert_eq!(Dispatch::Send(sqlite.1), sqlite.0);
}

#[test]
fn a_move_the_server_did_not_place_holds_the_next_one_until_a_sync_finds_it() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let first = queue(store, file_into(&m, MailboxRole::Archive));
        let second = queue(store, file_into(&m, MailboxRole::Trash));

        first_move_lands(store, first, None);
        let waiting = store.outbox_dispatch(second).unwrap();
        let listed = store.outbox_due(ACCOUNT, at(1)).unwrap();

        // The sync of Archive finds it, by its identity, under the UID the server gave it.
        deliver(store, &m, imap("Archive", 77));
        let found = store.outbox_dispatch(second).unwrap();
        let entry = store.outbox_due(ACCOUNT, at(1)).unwrap().remove(0);
        (
            waiting,
            listed.len(),
            listed[0].attempts,
            found,
            entry.attempts,
            store.remotes_of(m.id).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let (waiting, listed, attempts_waiting, found, attempts_after, now_at) = sqlite;
    assert_eq!(
        waiting,
        Dispatch::Wait,
        "sent to no address, or the old one"
    );
    assert_eq!(listed, 1, "still queued while it waits");
    assert_eq!(attempts_waiting, 0, "waiting is not a failed attempt");
    assert_eq!(
        found,
        Dispatch::Send(ProtoOp::SetMailbox {
            remotes: vec![imap("Archive", 77)],
            role: MailboxRole::Trash,
        })
    );
    assert_eq!(attempts_after, 0);
    assert_eq!(now_at, vec![imap("Archive", 77)]);
}

#[test]
fn an_operation_made_while_the_server_has_not_said_where_a_message_went_waits_for_the_sync() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let first = queue(store, file_into(&m, MailboxRole::Archive));
        first_move_lands(store, first, None);

        // Made after the move went and before any sync: the message has no address here, but it
        // is not mail the server never held, so it is queued rather than kept to this client.
        let later = queue(store, file_into(&m, MailboxRole::Trash));
        let waiting = store.outbox_dispatch(later).unwrap();
        let listed = store.outbox_due(ACCOUNT, at(1)).unwrap().len();

        deliver(store, &m, imap("Archive", 77));
        (waiting, listed, store.outbox_dispatch(later).unwrap())
    });
    assert_eq!(sqlite, memory);
    let (waiting, listed, found) = sqlite;
    assert_eq!(waiting, Dispatch::Wait);
    assert_eq!(listed, 1);
    assert_eq!(
        found,
        Dispatch::Send(ProtoOp::SetMailbox {
            remotes: vec![imap("Archive", 77)],
            role: MailboxRole::Trash,
        })
    );
}

#[test]
fn a_flag_change_queued_behind_a_move_lands_on_the_moved_message() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let first = queue(store, file_into(&m, MailboxRole::Archive));
        let flag = queue(
            store,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: Some(ReadState::Read),
                star: Some(Star::Starred),
            },
        );
        first_move_lands(store, first, Some(imap("Archive", 77)));
        store.outbox_dispatch(flag).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert_eq!(
        sqlite,
        Dispatch::Send(ProtoOp::SetFlags {
            remotes: vec![imap("Archive", 77)],
            read: Some(ReadState::Read),
            star: Some(Star::Starred),
        })
    );
}

#[test]
fn a_queued_graph_move_uses_the_id_the_move_before_it_returned() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, graph("INBOX", "AAMk-old"));
        let first = queue(store, file_into(&m, MailboxRole::Archive));
        let second = queue(store, file_into(&m, MailboxRole::Trash));
        assert_eq!(
            remotes(&store.outbox_dispatch(first).unwrap()),
            vec![graph("INBOX", "AAMk-old")]
        );
        store
            .remap(
                ACCOUNT,
                &graph("INBOX", "AAMk-old"),
                &graph("Archive", "AAMk-new"),
            )
            .unwrap();
        store.outbox_settle(first, Settle::Ok, at(1)).unwrap();
        store.outbox_dispatch(second).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert_eq!(remotes(&sqlite), vec![graph("Archive", "AAMk-new")]);
}

#[test]
fn a_later_operation_on_a_waiting_message_waits_behind_it_and_others_do_not() {
    let (sqlite, memory) = both(|store| {
        let (m, n, other) = (message(1), message(2), message(3));
        deliver(store, &m, imap("INBOX", 10));
        deliver(store, &n, imap("INBOX", 11));
        deliver(store, &other, imap("INBOX", 12));
        let both_moved = queue(
            store,
            RemoteIntent::SetMailbox {
                messages: vec![m.id, n.id],
                role: MailboxRole::Archive,
            },
        );
        let n_read = queue(
            store,
            RemoteIntent::SetFlags {
                messages: vec![n.id],
                read: Some(ReadState::Read),
                star: None,
            },
        );
        let other_read = queue(
            store,
            RemoteIntent::SetFlags {
                messages: vec![other.id],
                read: Some(ReadState::Read),
                star: None,
            },
        );
        // An earlier move of `m` that no server said anything about.
        store.unmap(ACCOUNT, &imap("INBOX", 10)).unwrap();
        (
            store.outbox_dispatch(both_moved).unwrap(),
            store.outbox_dispatch(n_read).unwrap(),
            remotes(&store.outbox_dispatch(other_read).unwrap()),
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(
        sqlite.0,
        Dispatch::Wait,
        "one of its messages has no address"
    );
    // `n` has an address, but marking it read before the move that was queued first would
    // reorder two operations on one message.
    assert_eq!(sqlite.1, Dispatch::Wait);
    assert_eq!(sqlite.2, vec![imap("INBOX", 12)]);
}

#[test]
fn an_operation_whose_messages_are_all_gone_has_nothing_to_send() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let moved = queue(store, file_into(&m, MailboxRole::Archive));
        // Deleted on another device: gone from the only mailbox that held it, so gone here.
        let mut batch = ingest("INBOX");
        batch.gone.push(imap("INBOX", 10));
        store.ingest(ACCOUNT, batch).unwrap();
        let dispatch = store.outbox_dispatch(moved).unwrap();
        store.outbox_settle(moved, Settle::Ok, at(1)).unwrap();
        (dispatch, store.outbox_due(ACCOUNT, at(1)).unwrap().len())
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, (Dispatch::Moot, 0));
}

#[test]
fn an_entry_no_longer_queued_is_not_sent() {
    let (sqlite, memory) = both(|store| {
        let m = message(1);
        deliver(store, &m, imap("INBOX", 10));
        let moved = queue(store, file_into(&m, MailboxRole::Archive));
        store.outbox_settle(moved, Settle::Ok, at(1)).unwrap();
        store.outbox_dispatch(moved).unwrap()
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, Dispatch::Wait);
}

#[test]
fn a_send_is_dispatched_exactly_as_it_was_queued() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1), imap("INBOX", 10));
        let intent = RemoteIntent::Send {
            draft: DraftId::generate(),
            raw: BlobId::generate(),
            mail_from: "me@example.test".to_owned(),
            rcpt_to: vec!["ada@example.test".to_owned()],
        };
        let id = queue(store, intent);
        let queued = store.outbox_due(ACCOUNT, at(0)).unwrap().remove(0).op;
        (store.outbox_dispatch(id).unwrap(), queued)
    });
    // Each store queued a send of its own, with ids of its own.
    for (dispatched, queued) in [sqlite, memory] {
        assert_eq!(dispatched, Dispatch::Send(queued));
    }
}
