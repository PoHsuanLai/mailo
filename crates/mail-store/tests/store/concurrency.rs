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

use crate::support::*;
use chrono::Utc;
use mail_domain::*;
use mail_store::testing::{count, seed_account};
use mail_store::{SqliteStore, Store, StoreError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn store() -> Arc<SqliteStore> {
    Arc::new(sqlite())
}

/// `count` new messages from `from` seconds on, each in a thread of its own, as a POP3 pass
/// hands them over.
fn ingest_of(store: &SqliteStore, from: i64, count: i64) -> Ingest {
    let raw = store.blobs().put(b"body").unwrap();
    let messages = (0..count)
        .map(|i| {
            let n = from + i;
            let key = format!("c{n}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread: ThreadId::generate(),
                key: MessageKey::Rfc(key.clone()),
                date: at(n),
                subject: format!("subject {n}"),
                rfc_message_id: Some(key.clone()),
                body: Body::Present {
                    text: Some("body".to_owned()),
                    raw,
                },
                ..message(0)
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
        cursor: Some(SyncCursor::Pop),
        messages,
        ..ingest("INBOX")
    }
}

#[test]
fn a_sync_writing_while_the_window_reads_does_not_deadlock() {
    // The shape of the real program: one thread absorbing mail, another drawing a list. A
    // reentrant lock makes recursion on one thread safe and says nothing about two.
    let store = store();
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));

    let reader = {
        let (store, stop, reads) = (store.clone(), stop.clone(), reads.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // Exactly what the list pane does, plus what a badge does.
                let _ = store.threads(&query(Filter::All, 50), at(0)).unwrap();
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
                    .ingest(acct_account(), ingest_of(&store, b * 50, 50))
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
    let store = store();
    store
        .ingest(acct_account(), ingest_of(&store, 0, 200))
        .unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let reader = {
        let (store, stop, reads) = (store.clone(), stop.clone(), reads.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = store.threads(&query(Filter::All, 50), at(0)).unwrap();
                reads.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let writer = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 1..10 {
                store
                    .ingest(acct_account(), ingest_of(&store, b * 200, 200))
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
    let store = store();
    let a = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 0..10 {
                store
                    .ingest(acct_account(), ingest_of(&store, b * 20, 20))
                    .unwrap();
            }
        })
    };
    let b = {
        let store = store.clone();
        std::thread::spawn(move || {
            for b in 0..10 {
                store
                    .ingest(acct_account(), ingest_of(&store, 1_000 + b * 20, 20))
                    .unwrap();
            }
        })
    };
    a.join().expect("writer a panicked");
    b.join().expect("writer b panicked");

    assert_eq!(store.count(&Filter::All, at(0)).unwrap(), 400);
    let rows = count(&store, "remote_map");
    assert_eq!(rows, 400, "remote_map disagrees with the mailbox");
}

/// Reading while the writer is busy.
///
/// WAL lets a reader run while a writer commits, but only across *connections*: with one handle
/// behind a mutex, every query in the process would wait behind whatever was committing. The
/// store's read-only connections are what make the claim true of this struct.
mod a_reader_that_is_not_the_writer {
    use super::*;

    /// A file-backed store. In-memory has no readers on purpose: a second `:memory:` handle is a
    /// second, empty database rather than a second view of this one.
    fn on_disk() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
        seed_account(&store, acct_account(), "me@example.test");
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
            let _guard = writing.raw_connection();
            announced.store(true, Ordering::SeqCst);
            std::thread::sleep(held);
        });
        while !taken.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }

        // And the list is asked for while that is still true.
        let started = Instant::now();
        let listed = store.threads(&query(Filter::All, 50), Utc::now());
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
        let raw = store.blobs().put(b"bytes").unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread,
            key: MessageKey::Rfc("m@example.test".to_owned()),
            date: Utc::now(),
            subject: "written just now".to_owned(),
            rfc_message_id: Some("m@example.test".to_owned()),
            body: Body::Present {
                text: Some("body".to_owned()),
                raw,
            },
            ..message(0)
        };
        store
            .ingest(
                acct_account(),
                Ingest {
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
                        message,
                    }],
                    flags: vec![],
                    flag_colors: Vec::new(),
                    labels: vec![],
                    label_names: Vec::new(),
                    provider_threads: Vec::new(),
                    gone: vec![],
                },
            )
            .unwrap();

        let listed = store.threads(&query(Filter::All, 50), Utc::now()).unwrap();
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
        let store = store();
        assert!(store.labels(acct_account()).is_ok());
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

    /// One read the window makes, by name, so a slow one says which it was.
    type Read = (
        &'static str,
        Box<dyn Fn(&SqliteStore) -> Result<(), StoreError>>,
    );

    /// Every read the window makes while it draws, against a store holding one conversation.
    fn the_window_reads(thread: ThreadId, message: MessageId) -> Vec<Read> {
        let now = Utc::now();
        let inbox = mailbox("INBOX");
        vec![
            ("list_accounts", Box::new(|s| s.list_accounts().map(drop))),
            (
                "account_caps",
                Box::new(|s| s.account_caps(acct_account()).map(drop)),
            ),
            (
                "identities",
                Box::new(|s| s.identities(acct_account()).map(drop)),
            ),
            (
                "last_synced",
                Box::new(|s| s.last_synced(acct_account()).map(drop)),
            ),
            ("passes", Box::new(|s| s.passes(acct_account()).map(drop))),
            ("folders", Box::new(|s| s.folders(acct_account()).map(drop))),
            ("labels", Box::new(|s| s.labels(acct_account()).map(drop))),
            (
                "count",
                Box::new(move |s| s.count(&Filter::All, now).map(drop)),
            ),
            (
                "threads",
                Box::new(move |s| s.threads(&query(Filter::All, 50), now).map(drop)),
            ),
            ("thread", Box::new(move |s| s.thread(thread).map(drop))),
            ("message", Box::new(move |s| s.message(message).map(drop))),
            ("placed", Box::new(move |s| s.placed(message).map(drop))),
            (
                "remotes_of",
                Box::new(move |s| s.remotes_of(message).map(drop)),
            ),
            (
                "found_in",
                Box::new(move |s| s.found_in(&[thread]).map(drop)),
            ),
            (
                "message_by_rfc_id",
                Box::new(|s| {
                    s.message_by_rfc_id(acct_account(), "c0@example.test")
                        .map(drop)
                }),
            ),
            (
                "thread_by_rfc_id",
                Box::new(|s| {
                    s.thread_by_rfc_id(acct_account(), "c0@example.test")
                        .map(drop)
                }),
            ),
            (
                "holds",
                Box::new(|s| {
                    s.holds(
                        acct_account(),
                        &MessageKey::Rfc("c0@example.test".to_owned()),
                    )
                    .map(drop)
                }),
            ),
            ("drafts", Box::new(|s| s.drafts(acct_account()).map(drop))),
            (
                "templates",
                Box::new(|s| s.templates(acct_account()).map(drop)),
            ),
            ("views", Box::new(|s| s.views().map(drop))),
            ("follow_ups", Box::new(|s| s.follow_ups().map(drop))),
            ("opened", Box::new(|s| s.opened().map(drop))),
            ("rules", Box::new(|s| s.rules(acct_account()).map(drop))),
            (
                "vacation",
                Box::new(|s| s.vacation(acct_account()).map(drop)),
            ),
            ("contacts", Box::new(|s| s.contacts().map(drop))),
            (
                "contacts_matching",
                Box::new(|s| s.contacts_matching("ad", 5).map(drop)),
            ),
            (
                "contact",
                Box::new(|s| s.contact("ada@example.test").map(drop)),
            ),
            ("groups", Box::new(|s| s.groups().map(drop))),
            ("address_books", Box::new(|s| s.address_books().map(drop))),
            (
                "terms_with_prefix",
                Box::new(|s| s.terms_with_prefix("sub", 5).map(drop)),
            ),
            (
                "outbox_due",
                Box::new(move |s| s.outbox_due(acct_account(), now).map(drop)),
            ),
            (
                "outbox_next",
                Box::new(move |s| s.outbox_next(acct_account(), now).map(drop)),
            ),
            (
                "outbox_dispatch",
                Box::new(|s| s.outbox_dispatch(OutboxId::from_i64(1)).map(drop)),
            ),
            (
                "cursor",
                Box::new({
                    let inbox = inbox.clone();
                    move |s| s.cursor(&inbox).map(drop)
                }),
            ),
            (
                "remote_refs",
                Box::new({
                    let inbox = inbox.clone();
                    move |s| s.remote_refs(&inbox).map(drop)
                }),
            ),
            (
                "unfetched",
                Box::new(|s| s.unfetched(acct_account(), 50).map(drop)),
            ),
            (
                "unfetched_in",
                Box::new({
                    let inbox = inbox.clone();
                    move |s| s.unfetched_in(&inbox, None, 50).map(drop)
                }),
            ),
            (
                "folder_contents",
                Box::new({
                    let inbox = inbox.clone();
                    move |s| s.folder_contents(&inbox).map(drop)
                }),
            ),
            (
                "remote_parts_in",
                Box::new(move |s| s.remote_parts_in(&inbox, 50).map(drop)),
            ),
            ("offline", Box::new(|s| s.offline(acct_account()).map(drop))),
            (
                "unplaced_into",
                Box::new(move |s| s.unplaced_into(acct_account(), message).map(drop)),
            ),
            ("reparse_queue", Box::new(|s| s.reparse_queue().map(drop))),
            ("pgp_keys", Box::new(|s| s.pgp_keys().map(drop))),
            ("smime_certs", Box::new(|s| s.smime_certs().map(drop))),
            (
                "receipt_answer",
                Box::new(move |s| s.receipt_answer(message).map(drop)),
            ),
            (
                "invite_answer",
                Box::new(move |s| s.invite_answer(message).map(drop)),
            ),
        ]
    }

    #[test]
    fn no_read_the_window_makes_waits_for_a_write_in_progress() {
        // The freeze this guards: the window draws on the main thread, and every read it made
        // through the writer waited for whatever batch a sync was committing.
        let (store, _dir) = on_disk();
        store
            .ingest(acct_account(), ingest_of(&store, 0, 1))
            .unwrap();
        let listed = store.threads(&query(Filter::All, 1), Utc::now()).unwrap();
        let thread = listed.items[0].id;
        let message = store.thread(thread).unwrap().messages[0];

        // A write in progress on another thread: the writer taken, a transaction open, a row
        // changed and not committed. Held until every read has been made, or for long enough
        // that a read queued behind it fails the bound below rather than hanging the test.
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let taken = Arc::new(AtomicBool::new(false));
        let writer = {
            let (store, taken) = (store.clone(), taken.clone());
            std::thread::spawn(move || {
                let db = store.raw_connection();
                db.execute_batch(
                    "BEGIN IMMEDIATE;
                     UPDATE thread_summary SET subject = 'not committed';",
                )
                .unwrap();
                taken.store(true, Ordering::SeqCst);
                // Either way the transaction ends: the reads are done, or they waited too long.
                let _ = finished.recv_timeout(Duration::from_secs(3));
                db.execute_batch("ROLLBACK").unwrap();
            })
        };
        while !taken.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }

        // A read queued behind the writer waits out its whole transaction, which is held for
        // three seconds, so any bound well under that tells the two apart; a loaded machine
        // making a fast read slow does not reach a second.
        let bound = Duration::from_secs(1);
        let mut slow = Vec::new();
        for (name, read) in the_window_reads(thread, message) {
            let started = Instant::now();
            let answer = read(&store);
            let waited = started.elapsed();
            assert!(answer.is_ok(), "{name}: {answer:?}");
            if waited >= bound {
                slow.push(format!("{name} waited {waited:?}"));
            }
        }
        // And what they read is what was committed, not the write still open beside them.
        let subject = store
            .threads(&query(Filter::All, 1), Utc::now())
            .unwrap()
            .items[0]
            .subject
            .clone();
        // The writer may go now; a send to a writer that already timed out has no one to tell.
        let _ = done.send(());
        writer.join().unwrap();

        assert!(slow.is_empty(), "reads queued behind the writer: {slow:?}");
        assert_eq!(subject, "subject 0");
    }

    #[test]
    fn a_write_reads_its_own_transaction_through_the_reader() {
        // The other side of the fallback: a read made inside a write, on the thread holding the
        // writer, sees what that write has not committed yet.
        let (store, _dir) = on_disk();
        store
            .ingest(acct_account(), ingest_of(&store, 0, 1))
            .unwrap();
        let db = store.raw_connection();
        db.execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE thread_summary SET subject = 'not committed';",
        )
        .unwrap();
        let inside = store
            .threads(&query(Filter::All, 1), Utc::now())
            .unwrap()
            .items[0]
            .subject
            .clone();
        db.execute_batch("ROLLBACK").unwrap();
        drop(db);
        assert_eq!(inside, "not committed");
    }
}

/// No other thread's read keeps the thread the last reader is kept for from reading: not the
/// client's workers holding a snapshot each, and not a sync pass reading while they do, which
/// waits for one of theirs rather than taking the kept one.
#[test]
fn the_thread_the_last_reader_is_kept_for_never_waits_on_another_s_read() {
    use std::sync::mpsc::channel;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    seed_account(&store, acct_account(), "me@example.test");
    let (pinned, pinned_rx) = channel::<()>();
    let (release, released) = channel::<()>();
    let released = Arc::new(std::sync::Mutex::new(released));

    // As many workers as the client runs, each holding a snapshot until the end of the test.
    let workers: Vec<_> = (0..store.reader_count() - 1)
        .map(|_| {
            let (store, pinned, released) = (store.clone(), pinned.clone(), released.clone());
            std::thread::spawn(move || {
                store
                    .read_pinned(|| {
                        let _ = store.count(&Filter::All, at(0)).unwrap();
                        pinned.send(()).unwrap();
                        // One message releases one worker.
                        let _ = released.lock().unwrap().recv();
                    })
                    .unwrap();
            })
        })
        .collect();
    for _ in &workers {
        pinned_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    // A sync pass reads while every worker holds its reader, and holds what it gets.
    let (let_go, let_go_rx) = channel::<()>();
    let syncing = {
        let store = store.clone();
        std::thread::spawn(move || {
            store
                .read_pinned(|| {
                    let _ = store.count(&Filter::All, at(0)).unwrap();
                    let _ = let_go_rx.recv();
                })
                .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(100));

    let (read, read_rx) = channel::<u64>();
    let kept = {
        let store = store.clone();
        std::thread::spawn(move || {
            SqliteStore::reserve_reader_for_this_thread();
            read.send(store.count(&Filter::All, at(0)).unwrap())
                .unwrap();
        })
    };
    // Asked while every other read still holds its reader.
    let counted = read_rx.recv_timeout(Duration::from_secs(5));

    for _ in &workers {
        let _ = release.send(());
    }
    let _ = let_go.send(());
    for worker in workers {
        worker.join().unwrap();
    }
    syncing.join().unwrap();
    kept.join().unwrap();
    assert_eq!(counted, Ok(0), "the read waited on another thread's reader");
}

/// An ingest larger than one chunk.
mod an_ingest_in_chunks {
    use super::*;
    use mail_store::testing::{INGEST_CHUNK, on_ingest_chunk};
    use std::sync::Mutex;

    /// What a reader saw after each chunk: how many conversations, and whether the cursor was
    /// written yet.
    type Seen = Arc<Mutex<Vec<(u64, bool)>>>;

    #[test]
    fn commits_each_chunk_and_the_cursor_last() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap();
        seed_account(&store, acct_account(), "me@example.test");
        let seen: Seen = Arc::default();
        let kept = seen.clone();
        on_ingest_chunk(&store, move |store| {
            let listed = store.count(&Filter::All, at(0)).unwrap();
            let cursor = store.cursor(&mailbox("INBOX")).unwrap().is_some();
            kept.lock().unwrap().push((listed, cursor));
        });

        let total = 2 * INGEST_CHUNK + 50;
        let mut batch = ingest_of(&store, 0, total as i64);
        // And a flag on every message, which counts towards the chunks after the messages.
        batch.flags = batch
            .messages
            .iter()
            .map(|m| (m.remote.clone(), ReadState::Read, Star::Starred))
            .collect();
        let patch = store.ingest(acct_account(), batch).unwrap();

        let total = total as u64;
        let chunk = INGEST_CHUNK as u64;
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (chunk, false),
                (2 * chunk, false),
                (total, false),
                (total, false),
                (total, true),
            ],
            "each chunk was not its own commit, or the cursor was written before the last"
        );
        // All of it, in the end, and the patch says all of it.
        assert_eq!(
            store
                .count(&Filter::Read(ReadState::Unread), at(0))
                .unwrap(),
            0
        );
        assert_eq!(patch.changes.len(), 3 * total as usize);
    }

    /// A later chunk that fails does not take the earlier chunks' arrivals with it: the error
    /// carries the patch of what stayed.
    #[test]
    fn that_stops_part_way_says_what_it_had_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap();
        seed_account(&store, acct_account(), "me@example.test");
        on_ingest_chunk(&store, |store| {
            store
                .raw_connection()
                .execute_batch(
                    "CREATE TRIGGER IF NOT EXISTS refuse_messages BEFORE INSERT ON messages \
                     BEGIN SELECT RAISE(ABORT, 'full'); END;",
                )
                .unwrap();
        });
        let batch = ingest_of(&store, 0, (INGEST_CHUNK + 50) as i64);
        let error = store.ingest(acct_account(), batch).unwrap_err();
        let written = error.written().expect("the error carries what was written");
        let upserts = written
            .changes
            .iter()
            .filter(|c| matches!(c, mail_domain::Change::MessageUpsert(_)))
            .count();
        assert_eq!(upserts, INGEST_CHUNK);
        assert_eq!(
            store.count(&Filter::All, at(0)).unwrap(),
            INGEST_CHUNK as u64
        );
    }

    /// A UIDVALIDITY reset is one commit however large: no reader sees the mailbox with its
    /// addresses dropped and its messages only partly there.
    #[test]
    fn a_reset_is_one_commit_whatever_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap();
        seed_account(&store, acct_account(), "me@example.test");
        let seen: Seen = Arc::default();
        let kept = seen.clone();
        on_ingest_chunk(&store, move |store| {
            let listed = store.count(&Filter::All, at(0)).unwrap();
            kept.lock().unwrap().push((listed, false));
        });
        let total = INGEST_CHUNK + 50;
        let mut batch = ingest_of(&store, 0, total as i64);
        batch.validity = mail_domain::UidValidity::Reset;
        store.ingest(acct_account(), batch).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![(total as u64, false)]);
    }

    #[test]
    fn an_empty_ingest_still_writes_its_cursor() {
        let store = sqlite();
        store
            .ingest(
                acct_account(),
                Ingest {
                    cursor: Some(SyncCursor::Pop),
                    ..ingest("INBOX")
                },
            )
            .unwrap();
        assert_eq!(
            store.cursor(&mailbox("INBOX")).unwrap(),
            Some(SyncCursor::Pop)
        );
    }
}

/// The window's reads are kept by name in `the_window_reads`, and a list kept by hand misses the
/// read nobody added to it. This is the rule it checks, held at the source: a function of the
/// SQLite store that only reads takes [`SqliteStore::reader`] and never the writer, which a sync
/// holds for a whole chunk. A read made inside a write is handed the writer by `reader` itself
/// (the thread already holds it), so there is no read the rule has to leave out.
///
/// A function counts as writing when it names a statement that changes the database, runs one
/// (`execute`, a transaction), or touches a topic; one that hands the writer out (`-> Writer`)
/// is the writer's own door.
#[test]
fn a_store_function_that_only_reads_never_takes_the_writer() {
    const WRITES: [&str; 14] = [
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "REPLACE ",
        "CREATE ",
        "DROP ",
        "ALTER ",
        "PRAGMA",
        ".execute",
        "execute_batch",
        "transaction(",
        ".touch(",
        "write_change",
        "VACUUM",
    ];
    let is_fn = |line: &str| {
        let line = line.trim_start();
        [
            "fn ",
            "pub fn ",
            "pub(super) fn ",
            "pub(crate) fn ",
            "async fn ",
            "pub async fn ",
        ]
        .iter()
        .any(|opening| line.starts_with(opening))
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sqlite");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    assert!(
        files.len() > 10,
        "the scan found {} files in {dir:?}",
        files.len()
    );
    let mut offenders = Vec::new();
    let mut functions = 0;
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        // What follows a `#[cfg(test)]` is the file's own tests, which may take either.
        let source: Vec<&str> = text
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect();
        let starts: Vec<usize> = (0..source.len()).filter(|i| is_fn(source[*i])).collect();
        for (n, start) in starts.iter().enumerate() {
            let end = starts.get(n + 1).copied().unwrap_or(source.len());
            functions += 1;
            let code: Vec<&str> = source[*start..end]
                .iter()
                .copied()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect();
            let body = code.join("\n");
            // Down to the first line that opens the body.
            let mut signature = String::new();
            for line in &code {
                signature.push_str(line);
                if line.contains('{') {
                    break;
                }
            }
            if body.contains(".connection()")
                && !signature.contains("Writer")
                && !WRITES.iter().any(|write| body.contains(write))
            {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    start + 1,
                    source[*start].trim()
                ));
            }
        }
    }
    assert!(functions > 100, "the scan read only {functions} functions");
    assert!(
        offenders.is_empty(),
        "these only read, and take the writer: use `self.reader()`\n{}",
        offenders.join("\n")
    );
}
