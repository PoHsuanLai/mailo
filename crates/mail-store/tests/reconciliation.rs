//! The rule that makes optimistic apply safe, and the sync edges around it.
//!
//! These are the tests the `sqlite` brief exists for. The failure they guard against is not a
//! crash — it is a star that flips back under the user's cursor one poll after they set it,
//! or a message that silently duplicates itself the first time Gmail is synced.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{Settle, SqliteStore, Store};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

struct Fixture {
    store: SqliteStore,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    Fixture { store, _dir: dir }
}

fn blob(f: &Fixture, bytes: &[u8]) -> BlobId {
    f.store.blobs().put(&f.store.connection(), bytes).unwrap()
}

fn message(f: &Fixture, thread: ThreadId, key: &str, n: i64) -> Message {
    Message {
        id: MessageId::generate(),
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc(key.to_owned()),
        date: at(n),
        from: Address {
            name: None,
            email: "sender@example.test".into(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".into(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: format!("Subject {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(key.to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(format!("body {n}")),
            raw: blob(f, format!("raw {n}").as_bytes()),
        },
        attachments: vec![],
    }
}

fn imap(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.into(),
        uidvalidity: 1,
        uid,
    }
}

fn ingest_of(mailbox: &str, messages: Vec<Fetched>) -> Ingest {
    Ingest {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: mailbox.into(),
        },
        validity: UidValidity::Same,
        cursor: Some(SyncCursor::Imap {
            uidvalidity: 1,
            uidnext: 100,
            modseq: None,
        }),
        messages,
        flags: vec![],
        labels: vec![],
        label_names: Vec::new(),
        gone: vec![],
    }
}

fn fetched(m: &Message, remote: RemoteRef) -> Fetched {
    Fetched {
        remote,
        key: m.key.clone(),
        raw: m.body.raw().expect("fixture has a body"),
        message: m.clone(),
    }
}

/// The Gmail case that a `RemoteRef`-as-identity design gets wrong: one message, two mailboxes,
/// two different uids. It must be ONE message with TWO mappings, not two messages.
#[test]
fn one_message_in_two_mailboxes_is_not_duplicated() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "shared@example.test", 1);

    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 11))]),
        )
        .unwrap();
    // Same message, different mailbox, different uid — as Gmail really reports it.
    f.store
        .ingest(
            ACCOUNT,
            ingest_of(
                "[Gmail]/All Mail",
                vec![fetched(&m, imap("[Gmail]/All Mail", 907))],
            ),
        )
        .unwrap();

    let count: i64 = f
        .store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "identity is MessageKey, not RemoteRef");

    let maps: i64 = f
        .store
        .connection()
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap();
    assert_eq!(maps, 2, "both addresses must resolve to that one message");
}

/// The flicker bug. A pending local change must survive an ingest that contradicts it.
#[test]
fn server_truth_does_not_overwrite_a_pending_local_change() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "a@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();

    // The user stars it. Applied locally, queued for the server.
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::MessageStar(m.id, Star::Starred)],
    };
    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::MessageStar(m.id, Star::Unstarred)],
    };
    f.store.apply(ACCOUNT, &patch).unwrap();
    let queued = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: None,
                star: Some(Star::Starred),
            },
            &undo,
            at(10),
        )
        .unwrap();
    assert!(
        queued.is_some(),
        "the message has a remote address, so this must queue"
    );

    // A poll lands before the server heard about it, still reporting unstarred.
    let mut stale = ingest_of("INBOX", vec![]);
    stale.flags = vec![(imap("INBOX", 5), ReadState::Unread, Star::Unstarred)];
    f.store.ingest(ACCOUNT, stale).unwrap();

    assert_eq!(
        f.store.message(m.id).unwrap().star,
        Star::Starred,
        "the pending star must be re-layered on top of server truth"
    );
    assert_eq!(f.store.thread(thread).unwrap().summary.star, Star::Starred);
}

/// Once the server confirms, the change stops being pending — otherwise it would be re-layered
/// forever and a later un-star from another client could never land.
#[test]
fn a_confirmed_change_stops_being_re_layered() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "b@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();

    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::MessageStar(m.id, Star::Unstarred)],
    };
    f.store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageStar(m.id, Star::Starred)],
            },
        )
        .unwrap();
    let id = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: None,
                star: Some(Star::Starred),
            },
            &undo,
            at(10),
        )
        .unwrap()
        .unwrap();

    f.store.outbox_settle(id, Settle::Ok, at(20)).unwrap();

    // Someone un-stars it elsewhere. That must now win.
    let mut later = ingest_of("INBOX", vec![]);
    later.flags = vec![(imap("INBOX", 5), ReadState::Unread, Star::Unstarred)];
    f.store.ingest(ACCOUNT, later).unwrap();

    assert_eq!(f.store.message(m.id).unwrap().star, Star::Unstarred);
}

/// A permanently refused operation must not leave the user looking at a state that will never
/// become true.
#[test]
fn a_fatal_failure_undoes_the_optimistic_change() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "c@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();

    f.store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageMailbox(m.id, MailboxRole::Archive)],
            },
        )
        .unwrap();
    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::MessageMailbox(m.id, MailboxRole::Inbox)],
    };
    let id = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetMailbox {
                messages: vec![m.id],
                role: MailboxRole::Archive,
            },
            &undo,
            at(10),
        )
        .unwrap()
        .unwrap();
    assert_eq!(f.store.message(m.id).unwrap().mailbox, MailboxRole::Archive);

    f.store
        .outbox_settle(
            id,
            Settle::Failed {
                reason: "message no longer exists".into(),
                retry: Retry::Fatal("gone".into()),
            },
            at(20),
        )
        .unwrap();

    assert_eq!(
        f.store.message(m.id).unwrap().mailbox,
        MailboxRole::Inbox,
        "undo must be applied"
    );
    assert_eq!(
        f.store.thread(thread).unwrap().summary.mailboxes,
        MailboxSet::only(MailboxRole::Inbox)
    );
    let left: i64 = f
        .store
        .connection()
        .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "a fatal entry must not be retried forever");
}

/// Vanishing from INBOX is what archiving looks like on Gmail, so a message is only really
/// gone once no mailbox holds it.
#[test]
fn expunge_drops_the_mapping_and_only_then_the_message() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "d@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();
    f.store
        .ingest(
            ACCOUNT,
            ingest_of(
                "[Gmail]/All Mail",
                vec![fetched(&m, imap("[Gmail]/All Mail", 99))],
            ),
        )
        .unwrap();

    let mut gone = ingest_of("INBOX", vec![]);
    gone.gone = vec![imap("INBOX", 5)];
    f.store.ingest(ACCOUNT, gone).unwrap();
    assert!(
        f.store.message(m.id).is_ok(),
        "still in All Mail, so still a message"
    );

    let mut gone_too = ingest_of("[Gmail]/All Mail", vec![]);
    gone_too.gone = vec![imap("[Gmail]/All Mail", 99)];
    f.store.ingest(ACCOUNT, gone_too).unwrap();
    assert!(f.store.message(m.id).is_err(), "no mailbox holds it now");
}

/// A UIDVALIDITY reset makes every uid for that mailbox meaningless at once.
#[test]
fn a_uidvalidity_reset_invalidates_the_mailbox_mapping() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "e@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();

    let mut reset = ingest_of("INBOX", vec![]);
    reset.validity = UidValidity::Reset;
    f.store.ingest(ACCOUNT, reset).unwrap();

    let maps: i64 = f
        .store
        .connection()
        .query_row(
            "SELECT count(*) FROM remote_map WHERE mailbox = 'INBOX'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(maps, 0, "old uids must not be reused");
    assert!(
        f.store.message(m.id).is_ok(),
        "the message itself is still real"
    );
}

/// A message with no server address cannot be told to the server, and saying so with `None`
/// beats queueing an operation whose recipient list is empty.
#[test]
fn enqueue_returns_none_when_nothing_is_addressable() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "local@example.test", 1);
    f.store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(m.clone()))],
            },
        )
        .unwrap();

    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };
    let queued = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: Some(ReadState::Read),
                star: None,
            },
            &undo,
            at(10),
        )
        .unwrap();
    assert!(queued.is_none());
}

/// One message in two mailboxes must be flagged in both, or the next sync reports it unread
/// again from whichever mailbox was missed.
#[test]
fn an_intent_resolves_to_every_address_of_its_messages() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "f@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();
    f.store
        .ingest(
            ACCOUNT,
            ingest_of(
                "[Gmail]/All Mail",
                vec![fetched(&m, imap("[Gmail]/All Mail", 99))],
            ),
        )
        .unwrap();

    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };
    let id = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: Some(ReadState::Read),
                star: None,
            },
            &undo,
            at(10),
        )
        .unwrap()
        .unwrap();

    let due = f.store.outbox_due(ACCOUNT, at(11)).unwrap();
    let entry = due.iter().find(|e| e.id == id).unwrap();
    match &entry.op {
        ProtoOp::SetFlags { remotes, .. } => {
            assert_eq!(remotes.len(), 2, "both mailboxes: {remotes:?}")
        }
        other => panic!("expected SetFlags, got {other:?}"),
    }
}

/// Two operations on one thread must reach the server in the order the user performed them.
#[test]
fn the_outbox_drains_in_insertion_order() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "g@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();
    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };

    let first = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: Some(ReadState::Read),
                star: None,
            },
            &undo,
            at(10),
        )
        .unwrap()
        .unwrap();
    let second = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetMailbox {
                messages: vec![m.id],
                role: MailboxRole::Archive,
            },
            &undo,
            at(9),
        )
        .unwrap()
        .unwrap();

    // Note `second` has an EARLIER next_attempt. Insertion order must still win.
    let due = f.store.outbox_due(ACCOUNT, at(20)).unwrap();
    assert_eq!(
        due.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![first, second]
    );
}

/// A retryable failure backs off rather than spinning, and keeps the change pending.
#[test]
fn a_retryable_failure_backs_off_and_stays_queued() {
    let f = fixture();
    let thread = ThreadId::generate();
    let m = message(&f, thread, "h@example.test", 1);
    f.store
        .ingest(
            ACCOUNT,
            ingest_of("INBOX", vec![fetched(&m, imap("INBOX", 5))]),
        )
        .unwrap();
    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };
    let id = f
        .store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![m.id],
                read: Some(ReadState::Read),
                star: None,
            },
            &undo,
            at(10),
        )
        .unwrap()
        .unwrap();

    f.store
        .outbox_settle(
            id,
            Settle::Failed {
                reason: "connection reset".into(),
                retry: Retry::Now,
            },
            at(20),
        )
        .unwrap();

    assert!(
        f.store.outbox_due(ACCOUNT, at(20)).unwrap().is_empty(),
        "must not be due immediately"
    );
    let later = f.store.outbox_due(ACCOUNT, at(10_000)).unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].attempts, 1);
}

/// `remote_map` grew a row per message per sync, on every protocol.
mod remote_map_identity {
    use super::*;

    #[test]
    fn syncing_the_same_mailbox_twice_does_not_duplicate_its_rows() {
        // SQLite treats NULLs as distinct in a PRIMARY KEY, and every remote_map row has one:
        // the CHECK constraint guarantees exactly one of uid/uidl is NULL. So the key never
        // conflicted, `INSERT OR REPLACE` had nothing to replace, and the table grew for ever.
        let f = fixture();
        let thread = ThreadId::generate();
        let message = message(&f, thread, "m1@example.test", 1);

        for _ in 0..3 {
            f.store
                .ingest(
                    ACCOUNT,
                    ingest_of("INBOX", vec![fetched(&message, imap("INBOX", 7))]),
                )
                .unwrap();
        }

        let rows: i64 = f
            .store
            .connection()
            .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "one message in one mailbox is one row");
    }

    #[test]
    fn a_pop_maildrop_does_not_duplicate_either() {
        // The POP3 row has *two* NULLs in its key, so it had the same hole — and the POP3
        // end-to-end test syncs once, which is exactly why it never showed.
        let f = fixture();
        let thread = ThreadId::generate();
        let message = message(&f, thread, "m2@example.test", 2);

        for _ in 0..3 {
            f.store
                .ingest(
                    ACCOUNT,
                    ingest_of(
                        "INBOX",
                        vec![fetched(
                            &message,
                            RemoteRef::Pop {
                                uidl: "UID-1".to_owned(),
                            },
                        )],
                    ),
                )
                .unwrap();
        }

        let rows: i64 = f
            .store
            .connection()
            .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn one_message_in_two_mailboxes_is_still_two_rows() {
        // The many-to-one mapping is the point of this table: a Gmail message marked read must
        // be marked read in INBOX *and* in All Mail. Deduplicating must not collapse that.
        let f = fixture();
        let thread = ThreadId::generate();
        let message = message(&f, thread, "m3@example.test", 3);

        f.store
            .ingest(
                ACCOUNT,
                ingest_of("INBOX", vec![fetched(&message, imap("INBOX", 7))]),
            )
            .unwrap();
        f.store
            .ingest(
                ACCOUNT,
                ingest_of("All Mail", vec![fetched(&message, imap("All Mail", 91))]),
            )
            .unwrap();

        let rows: i64 = f
            .store
            .connection()
            .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "the many-to-one mapping was collapsed");
    }
}
