//! Two threads on one store, which is how the application actually runs.
//!
//! `SqliteStore` holds **one** connection behind a `ReentrantMutex`, and the shell shares it by
//! `Arc` between the UI thread and a sync running on `spawn_blocking`. Every other test here
//! uses it from one thread, so the contention that exists in the real program has never
//! happened in a test.
//!
//! What is asserted is not throughput. It is that concurrent use terminates, does not panic, and
//! leaves the database consistent — a deadlock here is a window that stops repainting while mail
//! arrives, with nothing logged and nothing to see.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

fn ingest_of(store: &SqliteStore, from: i64, count: i64) -> Ingest {
    let raw = store.blobs().put(&store.connection(), b"body").unwrap();
    let messages = (0..count)
        .map(|i| {
            let n = from + i;
            let key = format!("c{n}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread: ThreadId::generate(),
                account: ACCOUNT,
                key: MessageKey::Rfc(key.clone()),
                date: at(n),
                from: Address {
                    name: None,
                    email: "s@example.test".to_owned(),
                },
                reply_to: vec![],
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: format!("subject {n}"),
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(key.clone()),
                read: ReadState::Unread,
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present {
                    text: Some("body".to_owned()),
                    raw,
                },
                attachments: vec![],
            };
            Fetched {
                remote: RemoteRef::Pop { uidl: key },
                key: message.key.clone(),
                raw,
                message,
            }
        })
        .collect();
    Ingest {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        validity: UidValidity::Same,
        cursor: Some(SyncCursor::Pop),
        messages,
        flags: vec![],
        labels: vec![],
        label_names: Vec::new(),
        gone: vec![],
    }
}

fn query(limit: u32) -> Query {
    Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    }
}

#[test]
fn a_sync_writing_while_the_window_reads_does_not_deadlock() {
    // The shape of the real program: one thread absorbing mail, another drawing a list. A
    // reentrant lock makes recursion on one thread safe and says nothing about two.
    let (store, _dir) = store();
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));

    let reader = {
        let (store, stop, reads) = (store.clone(), stop.clone(), reads.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // Exactly what the list pane does, plus what a badge does.
                let _ = store.threads(&query(50), at(0)).unwrap();
                let _ = store.count(&Filter::All, at(0)).unwrap();
                reads.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 0..20 {
                store
                    .ingest(ACCOUNT, ingest_of(&store, b * 50, 50))
                    .unwrap();
            }
        })
    };

    // A deadlock is the failure this guards, so it must not be waited on for ever.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !writer.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the writer never finished: a reader and a writer are deadlocked on the connection"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    writer.join().expect("the writing thread panicked");
    stop.store(true, Ordering::SeqCst);
    reader.join().expect("the reading thread panicked");

    assert_eq!(store.count(&Filter::All, at(0)).unwrap(), 1_000);
    assert!(
        reads.load(Ordering::SeqCst) > 0,
        "the reader never got the connection at all"
    );
}

#[test]
fn a_reader_is_not_starved_by_a_writer() {
    // One connection means reads and writes take turns. Turns are fine; never getting one is a
    // window that stops repainting for the length of a sync.
    let (store, _dir) = store();
    store.ingest(ACCOUNT, ingest_of(&store, 0, 200)).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = {
        let (store, stop, reads) = (store.clone(), stop.clone(), reads.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = store.threads(&query(50), at(0)).unwrap();
                reads.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 1..10 {
                store
                    .ingest(ACCOUNT, ingest_of(&store, b * 200, 200))
                    .unwrap();
            }
        })
    };
    writer.join().expect("the writing thread panicked");
    stop.store(true, Ordering::SeqCst);
    reader.join().expect("the reading thread panicked");

    let got = reads.load(Ordering::SeqCst);
    eprintln!("reads completed while nine batches were written: {got}");
    assert!(got > 5, "the reader managed only {got} reads during a sync");
}

#[test]
fn two_writers_do_not_corrupt_the_mailbox() {
    // Not the shape the application takes today — one engine per account — but the store is
    // behind an `Arc` and nothing stops it. Each message must appear once.
    let (store, _dir) = store();
    let a = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 0..10 {
                store
                    .ingest(ACCOUNT, ingest_of(&store, b * 20, 20))
                    .unwrap();
            }
        })
    };
    let b = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 0..10 {
                store
                    .ingest(ACCOUNT, ingest_of(&store, 1_000 + b * 20, 20))
                    .unwrap();
            }
        })
    };
    a.join().expect("writer a panicked");
    b.join().expect("writer b panicked");

    assert_eq!(store.count(&Filter::All, at(0)).unwrap(), 400);
    let rows: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 400, "remote_map disagrees with the mailbox");
}

/// Reading while the writer is busy — `plan.md` phase 8b.
///
/// The pragmas have said since migration 0001 that "WAL lets a reader run while a writer commits,
/// which is what keeps the UI responsive during a sync". That is true of SQLite and was false of
/// this struct: WAL's concurrent readers are concurrent across *connections*, and there was one,
/// behind a mutex, so every query in the process waited behind whatever was committing. The
/// single case the comment named was the single case it did not cover.
mod a_reader_that_is_not_the_writer {
    use super::*;

    /// A file-backed store. In-memory has no readers on purpose: a second `:memory:` handle is a
    /// second, empty database rather than a second view of this one.
    fn on_disk() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();
        (store, dir)
    }

    #[test]
    fn a_query_does_not_wait_for_a_write_to_finish() {
        let (store, _dir) = on_disk();
        let held = Duration::from_millis(600);

        // One thread takes the writer and keeps it, as an ingest batch does.
        let writing = store.clone();
        let taken = Arc::new(AtomicBool::new(false));
        let announced = taken.clone();
        let writer = std::thread::spawn(move || {
            let _guard = writing.connection();
            announced.store(true, Ordering::SeqCst);
            std::thread::sleep(held);
        });
        while !taken.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }

        // And the list is asked for while that is still true.
        let started = Instant::now();
        let listed = store.threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            Utc::now(),
        );
        let waited = started.elapsed();
        writer.join().unwrap();

        assert!(listed.is_ok(), "{listed:?}");
        // Compared against the write it ran beside rather than against a fixed number of
        // milliseconds: what is being asserted is that the query did not wait for the writer,
        // and "less than the time the writer held it" says exactly that on any machine.
        assert!(
            waited < held,
            "the query waited {waited:?} for a write that took {held:?}, \
             so it was still queued behind it"
        );
    }

    #[test]
    fn a_reader_sees_what_has_been_committed() {
        // The other half, and the one that would make the first test worthless on its own: a
        // connection that answered quickly with stale rows would pass it.
        let (store, _dir) = on_disk();
        let thread = ThreadId::generate();
        let raw = store.blobs().put(&store.connection(), b"bytes").unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc("m@example.test".to_owned()),
            date: Utc::now(),
            from: Address {
                name: None,
                email: "ada@example.test".to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "written just now".to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some("m@example.test".to_owned()),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some("body".to_owned()),
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
                    cursor: Some(SyncCursor::Pop),
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
                    label_names: Vec::new(),
                    gone: vec![],
                },
            )
            .unwrap();

        let listed = store
            .threads(
                &Query {
                    filter: Filter::All,
                    sort: Sort {
                        property: Property::Date,
                        dir: SortDir::Desc,
                    },
                    page: PageReq {
                        after: None,
                        limit: 50,
                    },
                },
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            listed.items.len(),
            1,
            "a reader did not see a committed write"
        );
        assert_eq!(listed.items[0].subject, "written just now");
    }

    #[test]
    fn an_in_memory_store_still_works_with_no_readers_at_all() {
        // Every test in this repository uses one, and a second `:memory:` connection would be a
        // second empty database — so `reader()` has to fall back to the writer rather than
        // opening one.
        let (store, _dir) = store();
        assert!(store.labels(ACCOUNT).is_ok());
        assert!(
            store
                .threads(
                    &Query {
                        filter: Filter::All,
                        sort: Sort {
                            property: Property::Date,
                            dir: SortDir::Desc
                        },
                        page: PageReq {
                            after: None,
                            limit: 10
                        },
                    },
                    Utc::now(),
                )
                .is_ok()
        );
    }
}
