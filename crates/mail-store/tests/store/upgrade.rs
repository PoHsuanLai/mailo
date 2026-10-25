//! Upgrading a database that already has data in it.
//!
//! Every other test creates a fresh database, which runs each migration against an *empty*
//! schema, and a migration whose job is to repair or move data has nothing to repair. So each
//! migration that rewrites, moves or deletes rows has a case here: a database built at the
//! version before it, the smallest rows that show what it does, and what must be true once the
//! store has opened it. A migration that only adds a table, a column or an index has none.
//!
//! Rather than check in a binary fixture, each version is built by applying the migrations up to
//! it: the SQL is the fixture, and it cannot drift from the migration it describes.

use mail_domain::id::account_id_from_uuid;
use mail_store::migrate::{self, EXPECTED_VERSION, MIGRATIONS};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use rusqlite::Connection;

/// The account, thread and message [`seed`] writes.
const ACCOUNT: &str = "00000000-0000-4000-8000-0000000000a1";
const THREAD: &str = "00000000-0000-4000-8000-0000000000e1";
const MESSAGE: &str = "00000000-0000-4000-8000-0000000000f1";
/// A second message, for the cases that need one beside [`MESSAGE`].
const OTHER: &str = "00000000-0000-4000-8000-0000000000f2";

/// Apply every migration up to and including `upto`, and nothing after it.
fn apply_upto(db: &Connection, upto: u32) {
    for (version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= upto) {
        db.execute_batch(sql).unwrap();
        if *version > 1 {
            db.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                [version],
            )
            .unwrap();
        }
    }
}

fn version_of(db: &Connection) -> u32 {
    db.query_row(
        "SELECT coalesce(max(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// The single number `sql` selects.
fn number(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn account() -> AccountId {
    account_id_from_uuid(ACCOUNT.parse().unwrap())
}

/// An account, a thread and a message in the columns version 1 has, so foreign keys are
/// satisfiable. The key and thread state are not what any build wrote, so the message does not
/// decode; a case that needs one that does writes real ones.
fn seed(db: &Connection) {
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at)
         VALUES (?1, 'me@example.test', '{}', datetime('now'))",
        [ACCOUNT],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads (id, account, snooze, pin) VALUES (?1, ?2, '\"none\"', '\"none\"')",
        [THREAD, ACCOUNT],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
             recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
             body_text, body_raw, attachments)
         VALUES (?1, ?2, ?3, '\"k\"', '2023-01-01T00:00:00Z', NULL, 'a@b.test', '{}', 's',
             NULL, '[]', NULL, '\"read\"', '\"unstarred\"', '\"inbox\"', NULL, NULL, '[]')",
        [MESSAGE, THREAD, ACCOUNT],
    )
    .unwrap();
}

/// How many messages the full-text index finds for `needle`.
fn hits(store: &SqliteStore, needle: &str) -> i64 {
    store
        .raw_connection()
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
            [needle],
            |r| r.get(0),
        )
        .unwrap()
}

/// One migration that moves rows: the version before it, what is on disk there, and what must
/// hold once the store has opened that database. `check` is handed the way to open it, so a case
/// can open it twice.
struct Case {
    migration: &'static str,
    from: u32,
    seed: fn(&Connection),
    check: fn(&dyn Fn() -> SqliteStore),
}

const CASES: &[Case] = &[
    Case {
        // Every version-1 database grew a remote_map row per message per sync. If the DELETE
        // misses one duplicate the unique index fails, and the application does not start.
        migration: "0002_remote_map_identity",
        from: 1,
        seed: |db| {
            seed(db);
            let map = |uidvalidity: Option<i64>, uid: Option<i64>, uidl: Option<&str>| {
                db.execute(
                    "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                     VALUES (?1, 'INBOX', ?2, ?3, ?4, ?5)",
                    rusqlite::params![ACCOUNT, uidvalidity, uid, uidl, MESSAGE],
                )
                .unwrap();
            };
            // Five syncs' worth of one IMAP mapping, and a POP-shaped one with two NULLs in its
            // key.
            (0..5).for_each(|_| map(Some(1), Some(101), None));
            (0..3).for_each(|_| map(None, None, Some("UID-1")));
            assert_eq!(number(db, "SELECT count(*) FROM remote_map"), 8);
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            // One POP mapping. The IMAP one was collapsed to one by 0002 and then dropped by
            // 0008, which removes every IMAP mapping for the header pass to rebuild.
            assert_eq!(number(&db, "SELECT count(*) FROM remote_map"), 1);
            let map = || {
                db.execute(
                    "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                     VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
                    [ACCOUNT, MESSAGE],
                )
            };
            map().unwrap();
            assert!(map().is_err(), "a duplicate mapping is refused");
        },
    },
    Case {
        // The index moved onto a `fts_text` Rust fills, which SQL cannot compute, so opening the
        // store backfills it: mail already on disk becomes findable, not only new mail.
        migration: "0004_fts_segmented",
        from: 3,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                     body_text, body_raw, attachments)
                 VALUES (?1, ?2, ?3, '\"k2\"', '2023-01-02T00:00:00Z', NULL, 'c@d.test', '{}',
                     '【重要】校園郵件信箱系統維護', NULL, '[]', NULL, '\"read\"', '\"unstarred\"',
                     '\"inbox\"', 'lunch on friday', NULL, '[]')",
                [OTHER, THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            assert_eq!(hits(&store, "lunch"), 1, "English mail on disk is findable");
            assert_eq!(hits(&store, "校園"), 1, "old Chinese mail was reindexed");
            assert_eq!(hits(&store, "維護"), 1);
            assert_eq!(
                hits(&store, "臺北"),
                0,
                "and not as a substring of everything"
            );
            drop(store);
            let again = open();
            let unfilled = "SELECT count(*) FROM messages WHERE fts_text IS NULL";
            assert_eq!(number(&again.raw_connection(), unfilled), 0);
        },
    },
    Case {
        // `To` and `Cc` joined the indexed text. A version-4 row already has `fts_text`, built
        // without them, so the migration clears the text and the index together; getting the
        // order wrong corrupts the index, hence the integrity checks.
        migration: "0005_fts_recipients",
        from: 4,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                     body_text, body_raw, attachments, fts_text)
                 VALUES (?1, ?2, ?3, '\"k2\"', '2023-01-02T00:00:00Z', NULL, 'c@d.test',
                     '{\"to\":[{\"name\":\"Ada Lovelace\",\"email\":\"ada@engine.test\"}],
                       \"cc\":[{\"name\":null,\"email\":\"grace@hopper.test\"}],
                       \"bcc\":[{\"name\":null,\"email\":\"secret@hidden.test\"}]}',
                     'minutes', NULL, '[]', NULL, '\"read\"', '\"unstarred\"',
                     '\"inbox\"', 'lunch on friday', NULL, '[]', 'minutes c d test lunch on friday')",
                [OTHER, THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let intact = || {
                store.raw_connection().execute(
                    "INSERT INTO messages_fts(messages_fts) VALUES ('integrity-check')",
                    [],
                )
            };
            assert_eq!(hits(&store, "lovelace"), 1, "a To name is indexed");
            assert_eq!(hits(&store, "hopper"), 1, "a Cc address is indexed");
            assert_eq!(hits(&store, "lunch"), 1, "what was findable still is");
            assert_eq!(
                hits(&store, "minutes"),
                1,
                "and the old entry is not left beside it"
            );
            assert_eq!(
                hits(&store, "hidden"),
                0,
                "a blind copy is never searchable"
            );
            intact().expect("the index agrees with the table after the upgrade");
            store
                .raw_connection()
                .execute(
                    "UPDATE messages SET read = '\"unread\"' WHERE id = ?1",
                    [OTHER],
                )
                .unwrap();
            intact().expect("the index agrees with the table after a write");
            assert_eq!(hits(&store, "hopper"), 1);
        },
    },
    Case {
        // A summary written before embedded images stopped counting is re-derived: the stored
        // count is what the paperclip and `has:attachment` read.
        migration: "0007_resummarize",
        from: 6,
        seed: |db| {
            seed(db);
            db.execute(
                r#"UPDATE messages SET msg_key = '{"kind":"rfc","v":"k@example.test"}',
                     attachments = '[{"name":"logo.png","mime":"image/png","size":3,
                       "blob":"04040404-0404-0404-0404-040404040404",
                       "inline":{"kind":"embedded","v":{"cid":"logo@x"}}}]'"#,
                [],
            )
            .unwrap();
            db.execute(
                "UPDATE threads SET snooze = ?1, pin = ?2",
                [
                    serde_json::to_string(&mail_domain::Snooze::Inactive).unwrap(),
                    serde_json::to_string(&mail_domain::Pin::Unpinned).unwrap(),
                ],
            )
            .unwrap();
            // What the old derivation wrote: one attachment, the embedded logo.
            db.execute(
                r#"INSERT INTO thread_summary (thread, account, subject, snippet, from_name,
                     from_email, participants, recipients, last_date, message_count, read, star,
                     mailboxes, labels, attachments, snooze, pin)
                   VALUES (?1, ?2, 's', '', NULL, 'a@b.test', '[]', '[]',
                     '2023-01-01T00:00:00.000000000Z', 1, '"read"', '"unstarred"', '["inbox"]',
                     '[]', '{"kind":"present","v":{"count":1}}', '"none"', '"none"')"#,
                [THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            let stored: String = db
                .query_row("SELECT attachments FROM thread_summary", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                stored, r#"{"kind":"none"}"#,
                "the embedded logo is not counted"
            );
            let queued = "SELECT count(*) FROM summaries_to_refresh";
            assert_eq!(number(&db, queued), 0, "the queue is emptied once worked");
        },
    },
    Case {
        // Both passes that run on open, the index backfill and the summary queue, read every
        // message, so either could lock someone out of all their mail over one bad row.
        migration: "0007_resummarize, a row that does not decode",
        from: 6,
        seed: |db| {
            seed(db);
            db.execute(
                "UPDATE messages SET recipients = 'not json', fts_text = NULL",
                [],
            )
            .unwrap();
            db.execute(
                r#"INSERT INTO thread_summary (thread, account, subject, snippet, from_name,
                     from_email, participants, recipients, last_date, message_count, read, star,
                     mailboxes, labels, attachments, snooze, pin)
                   VALUES (?1, ?2, 's', '', NULL, 'a@b.test', '[]', '[]',
                     '2023-01-01T00:00:00.000000000Z', 1, '"read"', '"unstarred"', '["inbox"]',
                     '[]', '{"kind":"none"}', '"none"', '"none"')"#,
                [THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            open();
        },
    },
    Case {
        // Every IMAP mapping goes, because any of them may name the wrong message; POP3's stay,
        // and so does the message.
        migration: "0008_remap_imap",
        from: 7,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                 VALUES (?1, 'INBOX', 1, 42, NULL, ?2), (?1, 'INBOX', NULL, NULL, 'u1', ?2)",
                [ACCOUNT, MESSAGE],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            let left: Vec<(Option<i64>, Option<String>)> = db
                .prepare("SELECT uid, uidl FROM remote_map")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(left, [(None, Some("u1".to_owned()))]);
            assert_eq!(number(&db, "SELECT count(*) FROM messages"), 1);
        },
    },
    Case {
        // Mail held before the address book is read into it once, on the first open: a sent
        // draft as written to, and its Sent-folder copy not counted again.
        migration: "0013_contacts",
        from: 12,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES ('00000000-0000-4000-8000-0000000000b1', ?1, 'Me', 'me@example.test',
                         '\"default\"')",
                [ACCOUNT],
            )
            .unwrap();
            db.execute(
                r#"INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                       recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                       body_text, body_raw, attachments)
                   VALUES (?1, ?2, ?3, '"k2"', '2023-01-02T00:00:00.000000000Z', 'Me',
                       'me@example.test', '{"to":[{"name":"Bob","email":"bob@example.test"}]}',
                       'Plans', NULL, '[]', NULL, '"read"', '"unstarred"', '"sent"', NULL, NULL,
                       '[]')"#,
                [OTHER, THREAD, ACCOUNT],
            )
            .unwrap();
            let state = serde_json::to_string(&mail_domain::SendState::Sent {
                at: chrono::DateTime::parse_from_rfc3339("2023-01-02T00:00:00Z")
                    .unwrap()
                    .to_utc(),
                message: None,
            })
            .unwrap();
            db.execute(
                r#"INSERT INTO drafts (id, account, identity, recipients, subject, in_reply_to,
                       forward_of, body_text, body_html, attachments, state, updated_at)
                   VALUES ('00000000-0000-4000-8000-0000000000d1', ?1,
                       '00000000-0000-4000-8000-0000000000b1',
                       '{"to":[{"name":"Bob Typed","email":"Bob@example.test"}],"cc":[],"bcc":[]}',
                       'Plans', NULL, NULL, '', NULL, '[]', ?2, '2023-01-02T00:00:00.000000000Z')"#,
                [ACCOUNT, &state],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let book = store.contacts().unwrap();
            let find = |address: &str| book.iter().find(|c| c.address == address).cloned();
            assert_eq!(find("a@b.test").map(|c| c.received.count), Some(1));
            let bob = find("bob@example.test").unwrap();
            assert_eq!(bob.written.count, 1, "the draft and its Sent copy are one");
            assert_eq!(bob.name.as_deref(), Some("Bob Typed"));
            assert_eq!(
                find("me@example.test").map(|c| c.kind),
                Some(mail_store::Kind::Own)
            );
            let pending = "SELECT count(*) FROM contacts_to_backfill";
            assert_eq!(number(&store.raw_connection(), pending), 0, "it runs once");
            drop(store);
            assert_eq!(
                open().contacts().unwrap(),
                book,
                "a second open counts nothing twice"
            );
        },
    },
    Case {
        // What body fetches drawn from two mailboxes at once left behind is undone, and nothing
        // else is touched.
        migration: "0015_unmix_body_batches",
        from: 14,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO blobs (id, hash, size, path, inline, created_at)
                 VALUES ('b', 'h', 1, NULL, x'00', datetime('now'))",
                [],
            )
            .unwrap();
            let message = |id: &str, key: &str, subject: &str, body: Option<&str>| {
                db.execute(
                    "INSERT INTO messages (id, thread, account, msg_key, date, from_name,
                         from_email, recipients, subject, in_reply_to, refs, rfc_message_id, read,
                         star, mailbox, body_text, body_raw, attachments)
                     VALUES (?1, ?2, ?3, ?4, '2023-01-02T00:00:00Z', NULL, 'a@b.test', '{}', ?5,
                         NULL, '[]', NULL, '\"read\"', '\"unstarred\"', '\"inbox\"', ?6,
                         CASE WHEN ?6 IS NULL THEN NULL ELSE 'b' END, '[]')",
                    rusqlite::params![id, THREAD, ACCOUNT, key, subject, body],
                )
                .unwrap();
            };
            let at = |mailbox: &str, uid: i64, message: &str| {
                db.execute(
                    "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                     VALUES (?1, ?2, 1, ?3, NULL, ?4)",
                    rusqlite::params![ACCOUNT, mailbox, uid, message],
                )
                .unwrap();
            };
            let rfc = |id: &str| format!("{{\"kind\":\"rfc\",\"v\":\"{id}@b.test\"}}");
            // Y answered for Sent UID 5 while INBOX was selected: X lost its address to it.
            message("x", &rfc("x"), "sent by me", None);
            message("y", &rfc("y"), "to me", Some("hello"));
            at("INBOX", 5, "y");
            at("Sent", 5, "y");
            // A message with no Message-ID, answered for Sent UID 6: stored again as a copy.
            let synthetic = |n: u8| format!("{{\"kind\":\"synthetic\",\"v\":[{n}]}}");
            message("n", &synthetic(1), "no id", Some("hi"));
            message("copy", &synthetic(9), "no id", Some("hi"));
            at("INBOX", 6, "n");
            at("Sent", 6, "copy");
            // A large message rebuilt around another's structure.
            let rebuilt =
                "--other\r\nContent-Type: application/pdf\r\nX-Mailo-Remote-Section: 2\r\n";
            message("big", &rfc("big"), "large", Some(rebuilt));
            at("Sent", 9, "big");
            // One held in two mailboxes under two UIDs, as a server keeps copies: untouched.
            message("both", &rfc("both"), "copied", Some("-- \r\nsignature"));
            at("INBOX", 7, "both");
            at("Projects", 8, "both");
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            let rows: Vec<(String, i64, String)> = db
                .prepare("SELECT mailbox, uid, message FROM remote_map ORDER BY mailbox, uid")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let row = |m: &str, u: i64, id: &str| (m.to_owned(), u, id.to_owned());
            assert_eq!(
                rows,
                vec![
                    row("INBOX", 7, "both"),
                    row("Projects", 8, "both"),
                    row("Sent", 9, "big"),
                ],
                "both addresses of one UID in two mailboxes go, and both copies with theirs"
            );
            let ids: Vec<String> = db
                .prepare("SELECT id FROM messages WHERE length(id) < 10 ORDER BY id")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            // Which of two identical digest-keyed messages is the copy cannot be told from
            // here, so both go; the header pass stores the one the server really holds again.
            assert_eq!(ids, ["big", "both", "x", "y"]);
            let body = |id: &str| -> (Option<String>, Option<String>) {
                db.query_row(
                    "SELECT body_text, body_raw FROM messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
            };
            assert_eq!(
                body("big"),
                (None, None),
                "the rebuilt body is fetched again"
            );
            assert_eq!(body("y"), (Some("hello".to_owned()), Some("b".to_owned())));
            assert_eq!(
                body("both"),
                (Some("-- \r\nsignature".to_owned()), Some("b".to_owned()))
            );
        },
    },
    Case {
        // An operation queued before the outbox kept its messages learns them from its pending
        // changes, and is sent to where the message is by then.
        migration: "0020_outbox_messages",
        from: 19,
        seed: |db| {
            use mail_domain::{ChangeId, Keyword, MailboxRole, Patch, ProtoOp};
            seed(db);
            db.execute(
                "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                 VALUES (?1, 'INBOX', 7, 10, NULL, ?2)",
                [ACCOUNT, MESSAGE],
            )
            .unwrap();
            let undo = serde_json::to_string(&Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            })
            .unwrap();
            let queue = |op: &ProtoOp| {
                db.execute(
                    "INSERT INTO outbox (account, op, undo, attempts, next_attempt, created_at)
                     VALUES (?1, ?2, ?3, 0, '2023-01-01T00:00:00.000000000Z',
                         '2023-01-01T00:00:00.000000000Z')",
                    rusqlite::params![ACCOUNT, serde_json::to_string(op).unwrap(), undo],
                )
                .unwrap();
                db.last_insert_rowid()
            };
            // A move, whose pending change names its message.
            let moved = queue(&ProtoOp::SetMailbox {
                remotes: vec![imap("INBOX", 10)],
                role: MailboxRole::Archive,
            });
            db.execute(
                "INSERT INTO pending_changes (message, outbox, changes) VALUES (?1, ?2, '[]')",
                rusqlite::params![MESSAGE, moved],
            )
            .unwrap();
            // A keyword, which has none.
            queue(&ProtoOp::AddKeyword {
                remotes: vec![imap("INBOX", 10)],
                keyword: Keyword::MdnSent,
            });
        },
        check: |open| {
            use mail_domain::{Keyword, MailboxRole, OutboxId, ProtoOp};
            use mail_store::Dispatch;
            let store = open();
            store
                .remap(account(), &imap("INBOX", 10), &imap("Archive", 77))
                .unwrap();
            assert_eq!(
                store.outbox_dispatch(OutboxId::from_i64(1)).unwrap(),
                Dispatch::Send(ProtoOp::SetMailbox {
                    remotes: vec![imap("Archive", 77)],
                    role: MailboxRole::Archive,
                }),
                "the move goes where the message is now"
            );
            assert_eq!(
                store.outbox_dispatch(OutboxId::from_i64(2)).unwrap(),
                Dispatch::Send(ProtoOp::AddKeyword {
                    remotes: vec![imap("INBOX", 10)],
                    keyword: Keyword::MdnSent,
                }),
                "the keyword is sent as it was queued"
            );
        },
    },
    Case {
        // A message waiting to be found before its wait was counted has no folder to be found
        // in, so only passes end the wait. The migration adds columns, but this row's wait would
        // otherwise never end.
        migration: "0021_unplaced_bound",
        from: 20,
        seed: |db| {
            use mail_domain::{ChangeId, MailboxRole, Patch, ProtoOp};
            seed(db);
            db.execute(
                "INSERT INTO unplaced (account, message) VALUES (?1, ?2)",
                [ACCOUNT, MESSAGE],
            )
            .unwrap();
            let op = ProtoOp::SetMailbox {
                remotes: vec![imap("INBOX", 10)],
                role: MailboxRole::Trash,
            };
            let undo = Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            };
            db.execute(
                "INSERT INTO outbox (account, op, undo, attempts, next_attempt, created_at,
                     messages)
                 VALUES (?1, ?2, ?3, 0, '2023-01-01T00:00:00.000000000Z',
                     '2023-01-01T00:00:00.000000000Z', ?4)",
                rusqlite::params![
                    ACCOUNT,
                    serde_json::to_string(&op).unwrap(),
                    serde_json::to_string(&undo).unwrap(),
                    format!("[\"{MESSAGE}\"]"),
                ],
            )
            .unwrap();
        },
        check: |open| {
            use mail_domain::OutboxId;
            use mail_store::{Dispatch, PASSES_TO_FIND, SYNCS_TO_FIND};
            let store = open();
            let counted: (Option<String>, u32, u32) = store
                .raw_connection()
                .query_row("SELECT mailbox, syncs, passes FROM unplaced", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .unwrap();
            assert_eq!(counted, (None, 0, 0), "where it went was never kept");
            let entry = OutboxId::from_i64(1);
            assert_eq!(store.outbox_dispatch(entry).unwrap(), Dispatch::Wait);
            let synced = vec!["INBOX".to_owned(), "Archive".to_owned()];
            for _ in 0..SYNCS_TO_FIND.max(PASSES_TO_FIND - 1) {
                store.unplaced_pass(account(), &synced).unwrap();
            }
            assert_eq!(store.outbox_dispatch(entry).unwrap(), Dispatch::Wait);
            store.unplaced_pass(account(), &synced).unwrap();
            assert!(matches!(
                store.outbox_dispatch(entry).unwrap(),
                Dispatch::Lost(_)
            ));
        },
    },
    Case {
        // An account of the desktop's accountd (auth `granted`) goes with everything that named
        // it, and one of mailo's own beside it stays. Its plan would no longer read otherwise.
        migration: "0031_drop_granted_accounts",
        from: 30,
        seed: |db| {
            const GRANTED: &str = "00000000-0000-4000-8000-0000000000a2";
            seed(db);
            db.execute(
                "UPDATE accounts SET plan = ?1",
                [serde_json::to_string(&own_plan()).unwrap()],
            )
            .unwrap();
            // What a linked build wrote for an account of accountd's.
            let granted_plan = r#"{"address":"them@example.test",
                "incoming":{"kind":"imap","v":{"host":"imap.example.test","port":993,"tls":"implicit"}},
                "outgoing":{"kind":"smtp","v":{"host":"smtp.example.test","port":465,"tls":"implicit"}},
                "auth":{"kind":"granted","v":{"account":"provider-them-example-test","grant":"grant-1",
                    "endpoints":[]}},
                "identities":[]}"#;
            db.execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'them@example.test', ?2, datetime('now'))",
                [GRANTED, granted_plan],
            )
            .unwrap();
            db.execute(
                "INSERT INTO threads (id, account, snooze, pin)
                 VALUES ('00000000-0000-4000-8000-0000000000e2', ?1, '\"none\"', '\"none\"')",
                [GRANTED],
            )
            .unwrap();
            db.execute(
                "INSERT INTO account_passes (account, finished_at)
                 VALUES (?1, '2026-10-01T00:00:00Z')",
                [GRANTED],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let listed = store.list_accounts().unwrap();
            let ids: Vec<String> = listed.iter().map(|a| a.id.to_string()).collect();
            assert_eq!(ids, [ACCOUNT], "only mailo's own account is left");
            assert_eq!(listed[0].plan.as_ref().unwrap(), &own_plan());
            let db = store.raw_connection();
            assert_eq!(
                number(&db, "SELECT count(*) FROM threads"),
                1,
                "its threads went"
            );
            assert_eq!(number(&db, "SELECT count(*) FROM account_passes"), 0);
            assert_eq!(
                number(&db, "SELECT count(*) FROM messages"),
                1,
                "the other's mail stays"
            );
        },
    },
    Case {
        // The list sorts by sender, subject, size, unread and starred, from columns every
        // existing summary gets from its newest message, not left at their defaults until the
        // thread is next written, which for old mail is never.
        migration: "0033_sort_keys",
        from: 32,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                     body_text, body_raw, attachments)
                 VALUES (?1, ?2, ?3, '\"k2\"', '2023-01-02T00:00:00Z', 'Ada', 'ada@b.test', '{}',
                     'Plans', NULL, '[]', NULL, '\"unread\"', '\"starred\"', '\"inbox\"',
                     'héllo', NULL,
                     '[{\"name\":\"a.pdf\",\"mime\":\"application/pdf\",\"size\":100,
                        \"remote_section\":\"2\",\"inline\":{\"kind\":\"attached\"}}]')",
                [OTHER, THREAD, ACCOUNT],
            )
            .unwrap();
            db.execute(
                "INSERT INTO thread_summary (thread, account, subject, snippet, from_name,
                     from_email, participants, recipients, last_date, message_count, read, star,
                     mailboxes, labels, attachments, snooze, pin)
                 VALUES (?1, ?2, 's', 'hello', 'Ada', 'ada@b.test', '[]', '[]',
                     '2023-01-02T00:00:00Z', 2, '\"unread\"', '\"starred\"', '[\"inbox\"]',
                     '[]', '{\"files\":1}', '\"none\"', '\"none\"')",
                [THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            let row: (String, String, i64, i64, i64) = db
                .query_row(
                    "SELECT sort_subject, sort_sender, size, unread, starred FROM thread_summary",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .unwrap();
            // The newer message's: 'héllo' is six bytes, and its part says 100.
            assert_eq!(row, ("plans".to_owned(), "ada".to_owned(), 106, 1, 1));
            assert_eq!(
                number(&db, "SELECT count(*) FROM summaries_to_refresh"),
                0,
                "the queue is emptied on open"
            );
        },
    },
];

fn imap(mailbox: &str, uid: u32) -> mail_domain::RemoteRef {
    mail_domain::RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 7,
        uid,
    }
}

/// A plan of mailo's own, as `presets::manual` writes one.
fn own_plan() -> mail_domain::AccountPlan {
    let manual = mail_domain::presets::Manual {
        imap_host: "imap.example.test".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.example.test".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    mail_domain::presets::manual("me@example.test", &manual, at).plan
}

#[test]
fn each_migration_that_moves_rows_keeps_them() {
    for case in CASES {
        // Printed so a failing assertion says which case it was.
        eprintln!("{} (from version {})", case.migration, case.from);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.db");
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
            apply_upto(&db, case.from);
            (case.seed)(&db);
        }
        let open = || SqliteStore::open(&path, dir.path()).unwrap();
        (case.check)(&open);
        assert_eq!(version_of(&open().raw_connection()), EXPECTED_VERSION);
    }
}

#[test]
fn an_empty_database_migrates_to_the_current_version() {
    let db = Connection::open_in_memory().unwrap();
    migrate::migrate(&db).unwrap();
    assert_eq!(version_of(&db), EXPECTED_VERSION);
}

#[test]
fn a_database_at_the_current_version_opens_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        apply_upto(&db, EXPECTED_VERSION);
        seed(&db);
        db.execute(
            "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
             VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
            [ACCOUNT, MESSAGE],
        )
        .unwrap();
    }
    let store = SqliteStore::open(&path, dir.path()).expect("a current database opens");
    let db = store.raw_connection();
    assert_eq!(version_of(&db), EXPECTED_VERSION);
    assert_eq!(
        number(&db, "SELECT count(*) FROM remote_map"),
        1,
        "and keeps its rows"
    );
}
