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
/// The identities 0034's case writes: signed, signed with only blank lines, and unsigned.
const SIGNED: &str = "00000000-0000-4000-8000-0000000000b1";
const BLANK: &str = "00000000-0000-4000-8000-0000000000b2";
const UNSIGNED: &str = "00000000-0000-4000-8000-0000000000b3";
/// The drafts 0038's case writes: plain, and with HTML.
const PLAIN_DRAFT: &str = "00000000-0000-4000-8000-0000000000d1";
const RICH_DRAFT: &str = "00000000-0000-4000-8000-0000000000d2";
/// The plain template 0038's case writes.
const PLAIN_TEMPLATE: &str = "00000000-0000-4000-8000-0000000000c1";
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
        // And a thread that does not rebuild is not forgotten: it stays queued, and is said, for
        // the next open to try again.
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
            let store = open();
            let queued = "SELECT count(*) FROM summaries_to_refresh";
            assert_eq!(
                number(&store.raw_connection(), queued),
                1,
                "the thread that did not rebuild stays queued"
            );
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
        // The one signature an identity had becomes its first row, the default for both, and
        // the column it was in is emptied; a blank one becomes nothing.
        migration: "0034_signatures",
        from: 32,
        seed: |db| {
            seed(db);
            for (id, email, signature) in [
                (
                    SIGNED,
                    "me@example.test",
                    Some("Ada Lovelace\nAnalytical Engines"),
                ),
                (BLANK, "alias@example.test", Some("  \n ")),
                (UNSIGNED, "other@example.test", None),
            ] {
                db.execute(
                    "INSERT INTO identities (id, account, from_name, from_email, reply_to,
                         signature, is_default)
                     VALUES (?1, ?2, NULL, ?3, NULL, ?4, '\"alternate\"')",
                    rusqlite::params![id, ACCOUNT, email, signature],
                )
                .unwrap();
            }
        },
        check: |open| {
            use mail_domain::{IdentityId, IsDefault, SignatureUse};
            let store = open();
            let id = |text: &str| IdentityId::from_uuid(text.parse().unwrap());
            let signed = store.signatures(id(SIGNED)).unwrap();
            assert_eq!(signed.len(), 1, "{signed:?}");
            let only = &signed[0];
            assert_eq!(only.text, "Ada Lovelace\nAnalytical Engines");
            assert_eq!(only.name, mail_domain::UNNAMED_SIGNATURE);
            assert_eq!(only.html, None);
            assert_eq!(
                (only.new_mail, only.replies),
                (IsDefault::Default, IsDefault::Default)
            );
            assert_eq!(
                store
                    .default_signature(id(SIGNED), SignatureUse::Replies)
                    .unwrap()
                    .map(|s| s.id),
                Some(only.id)
            );
            // Read through the identity as it always was.
            assert_eq!(
                store
                    .identity(id(SIGNED))
                    .unwrap()
                    .unwrap()
                    .signature
                    .as_deref(),
                Some("Ada Lovelace\nAnalytical Engines")
            );
            assert_eq!(store.signatures(id(BLANK)).unwrap(), vec![]);
            assert_eq!(store.signatures(id(UNSIGNED)).unwrap(), vec![]);
            assert_eq!(store.identity(id(BLANK)).unwrap().unwrap().signature, None);
            let db = store.raw_connection();
            assert_eq!(
                number(
                    &db,
                    "SELECT count(*) FROM identities WHERE signature IS NOT NULL"
                ),
                0,
                "the old column still speaks"
            );
            // Deleting the row leaves the identity unsigned: nothing comes back from the column.
            drop(db);
            store.delete_signature(only.id).unwrap();
            assert_eq!(store.identity(id(SIGNED)).unwrap().unwrap().signature, None);
        },
    },
    Case {
        // A plain draft an older build kept has its text as written, which is not flowed text,
        // so it is queued to be rewritten, as is a plain template; one with HTML the composer
        // wrote, as flowed text.
        migration: "0038_reflow_drafts",
        from: 37,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
                [SIGNED, ACCOUNT],
            )
            .unwrap();
            for (id, text, html) in [
                (PLAIN_DRAFT, "Dear Ada, \nFrom here", None),
                (RICH_DRAFT, "Dear Bea,", Some("<p>Dear Bea,</p>")),
            ] {
                db.execute(
                    r#"INSERT INTO drafts (id, account, identity, recipients, subject,
                           in_reply_to, forward_of, body_text, body_html, attachments, state,
                           updated_at)
                       VALUES (?1, ?2, ?3, '{"to":[],"cc":[],"bcc":[]}', 'Plans', NULL, NULL,
                           ?4, ?5, '[]', '{"kind":"editing"}',
                           '2023-01-02T00:00:00.000000000Z')"#,
                    rusqlite::params![id, ACCOUNT, SIGNED, text, html],
                )
                .unwrap();
            }
            db.execute(
                r#"INSERT INTO templates (id, account, identity, name, recipients, subject,
                       body_text, body_html, attachments, receipt, updated_at)
                   VALUES (?1, ?2, ?3, 'Weekly', '{"to":[],"cc":[],"bcc":[]}', 'Plans',
                       ' indented', NULL, '[]', '"unrequested"',
                       '2023-01-02T00:00:00.000000000Z')"#,
                [PLAIN_TEMPLATE, ACCOUNT, SIGNED],
            )
            .unwrap();
        },
        check: |open| {
            use mail_domain::{DraftId, TemplateId};
            use mail_store::Kept;
            let store = open();
            let draft = DraftId::from_uuid(PLAIN_DRAFT.parse().unwrap());
            let template = TemplateId::from_uuid(PLAIN_TEMPLATE.parse().unwrap());
            assert_eq!(
                store.reflow_queue().unwrap(),
                vec![
                    (Kept::Draft(draft), "Dear Ada, \nFrom here".to_owned()),
                    (Kept::Template(template), " indented".to_owned()),
                ]
            );
            store
                .reflowed(Kept::Draft(draft), "Dear Ada,\n From here")
                .unwrap();
            store
                .reflowed(Kept::Template(template), "  indented")
                .unwrap();
            drop(store);
            let store = open();
            assert_eq!(store.reflow_queue().unwrap(), vec![], "it runs once");
            assert_eq!(store.draft(draft).unwrap().text, "Dear Ada,\n From here");
            assert_eq!(store.template(template).unwrap().text, "  indented");
        },
    },
    Case {
        // The list sorts by sender, subject, size, unread and starred, from columns every
        // existing summary gets from its newest message, not left at their defaults until the
        // thread is next written, which for old mail is never.
        migration: "0041_sort_keys",
        from: 40,
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
            // The summary is rebuilt from these rows on open, so they are written as the store
            // reads them now: a row that did not decode would keep its thread queued.
            for (message, key) in [(MESSAGE, "k@example.test"), (OTHER, "k2@example.test")] {
                let stored = format!(r#"{{"kind":"rfc","v":"{key}"}}"#);
                db.execute(
                    "UPDATE messages SET msg_key = ?2 WHERE id = ?1",
                    [message, stored.as_str()],
                )
                .unwrap();
            }
            db.execute(
                "UPDATE threads SET snooze = ?1, pin = ?2",
                [
                    serde_json::to_string(&mail_domain::Snooze::Inactive).unwrap(),
                    serde_json::to_string(&mail_domain::Pin::Unpinned).unwrap(),
                ],
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
            // The newer message's, as the store reads it: it has no raw bytes, so it is held by
            // its headers and its 'héllo' is not counted, and its part says 100.
            assert_eq!(row, ("plans".to_owned(), "ada".to_owned(), 100, 1, 1));
            assert_eq!(
                number(&db, "SELECT count(*) FROM summaries_to_refresh"),
                0,
                "the queue is emptied on open"
            );
        },
    },
    Case {
        // Attachment names joined the indexed text. A version-32 row was indexed without them,
        // so the migration clears the text and the index together and the open backfills it.
        migration: "0042_fts_attachment_names",
        from: 41,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                     body_text, body_raw, attachments, fts_text)
                 VALUES (?1, ?2, ?3, '\"k2\"', '2023-01-02T00:00:00Z', NULL, 'c@d.test', '{}',
                     'minutes', NULL, '[]', NULL, '\"read\"', '\"unstarred\"',
                     '\"inbox\"', 'lunch on friday', NULL,
                     '[{\"name\":\"quarterly-figures.pdf\",\"mime\":\"application/pdf\",
                        \"size\":12345,\"blob\":\"04040404-0404-0404-0404-040404040404\",
                        \"inline\":{\"kind\":\"attached\"}}]',
                     'minutes c d test lunch on friday')",
                [OTHER, THREAD, ACCOUNT],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            assert_eq!(
                hits(&store, "quarterly"),
                1,
                "an attachment's name is indexed"
            );
            assert_eq!(hits(&store, "lunch"), 1, "what was findable still is");
            assert_eq!(hits(&store, "minutes"), 1, "and only once");
            store
                .raw_connection()
                .execute(
                    "INSERT INTO messages_fts(messages_fts) VALUES ('integrity-check')",
                    [],
                )
                .expect("the index agrees with the table after the upgrade");
        },
    },
    Case {
        // The VIPs are a table of their own: the book's rows stay as they were, nobody is a VIP
        // yet, and one can be marked at once.
        migration: "0043_vips",
        from: 42,
        seed: |db| {
            seed(db);
            db.execute(
                "INSERT INTO contacts (address, name, name_rank, written_count, received_count,
                     kind, origin, keys)
                 VALUES ('dana@example.test', 'Dana', 3, 0, 1, '\"person\"',
                     '{\"kind\":\"manual\"}', ' dana ')",
                [],
            )
            .unwrap();
        },
        check: |open| {
            let store = open();
            assert_eq!(store.vips().unwrap(), []);
            let kept = store.contact("dana@example.test").unwrap();
            assert_eq!(
                kept.and_then(|contact| contact.name).as_deref(),
                Some("Dana")
            );
            store.put_vip("Dana@example.test", Some("Dana")).unwrap();
            assert_eq!(
                store
                    .vips()
                    .unwrap()
                    .into_iter()
                    .map(|vip| vip.address)
                    .collect::<Vec<_>>(),
                ["dana@example.test"]
            );
        },
    },
    Case {
        // Each message's size is written beside it from its text and its parts, a row whose
        // parts are not JSON counting none, and the index is told of a message only when its
        // indexed text is written.
        migration: "0050_message_size",
        from: 49,
        seed: |db| {
            seed(db);
            for (id, key, body, parts) in [
                (OTHER, "k2", "héllo", r#"[{"size":100}]"#),
                (
                    "00000000-0000-4000-8000-0000000000f3",
                    "k3",
                    "plain",
                    "not json",
                ),
            ] {
                let key = format!("\"{key}\"");
                db.execute(
                    "INSERT INTO messages (id, thread, account, msg_key, date, from_name,
                         from_email, recipients, subject, in_reply_to, refs, rfc_message_id, read,
                         star, mailbox, body_text, body_raw, attachments)
                     VALUES (?1, ?2, ?3, ?4, '2023-01-02T00:00:00Z', NULL, 'c@d.test', '{}', 'sized',
                         NULL, '[]', NULL, '\"read\"', '\"unstarred\"', '\"inbox\"', ?5, NULL, ?6)",
                    [id, THREAD, ACCOUNT, key.as_str(), body, parts],
                )
                .unwrap();
            }
        },
        check: |open| {
            let store = open();
            let db = store.raw_connection();
            let sizes: Vec<(String, i64)> = db
                .prepare("SELECT id, size FROM messages ORDER BY id")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(
                sizes,
                [
                    (MESSAGE.to_owned(), 0),
                    // 'héllo' is six bytes, and its part says 100.
                    (OTHER.to_owned(), 106),
                    ("00000000-0000-4000-8000-0000000000f3".to_owned(), 5),
                ]
            );
            drop(db);
            let set = |sql: &str| {
                store.raw_connection().execute(sql, []).unwrap();
            };
            set(
                "UPDATE messages SET fts_text = 'zebra' WHERE id = '00000000-0000-4000-8000-0000000000f2'",
            );
            assert_eq!(hits(&store, "zebra"), 1);
            set("UPDATE messages SET read = '\"unread\"'");
            assert_eq!(hits(&store, "zebra"), 1, "a flag leaves the index alone");
            set(
                "UPDATE messages SET fts_text = 'aardvark' WHERE id = '00000000-0000-4000-8000-0000000000f2'",
            );
            assert_eq!(hits(&store, "zebra"), 0);
            assert_eq!(
                hits(&store, "aardvark"),
                1,
                "new indexed text replaces the old"
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

/// The SHA-256 of every migration file that has landed on master, 0001 to 0040. A database that
/// is past a version never runs that file again, so a change to it reaches no database that
/// matters: the change belongs in a new migration.
const LANDED_ON_MASTER: [(u32, &str); 40] = [
    (
        1,
        "d46fb00f007468c7d832c90ad6ebc0dd06c8f2a227019e99f9d43ce473d00422",
    ),
    (
        2,
        "266948f384fcad6417a3f42092b653b2d0b65f846f62f3767e3dece7af2fd8ae",
    ),
    (
        3,
        "9f3ed3eb625413bfa8797cc058a9172e10df3ea8c9daa23564c2e11dcc7394db",
    ),
    (
        4,
        "09cd53913efa769e1ac2854340bac4e188020b0b5f554f9d8d13869484ca91a3",
    ),
    (
        5,
        "fe5eddac5b8fcf7904df2aa4f8ff60cf56d9d4efb5929b11b444ba0d24c3872b",
    ),
    (
        6,
        "f35d41c6fdf32685f63fe40381be52f6c5c27e01fd210014c39ee067d92257b9",
    ),
    (
        7,
        "c1f22e531665b44a805ebdc939855260ad8b616b515449c31e7f4f1863037b76",
    ),
    (
        8,
        "b0d01eba85e02866ed1b8d67976c2f00e389bb06490d95acd3cd7a5fd0b23902",
    ),
    (
        9,
        "6643ae80846e0243d165ccf6e27cd5fd3e4c562b895c7f3cc7f4a5d4432a63fe",
    ),
    (
        10,
        "b8f905f16efa9ef31035c5e5e3870df571f540c5fa006a8d68dcfcb0ed42730e",
    ),
    (
        11,
        "a971286deebc90d98ecf94652228ff5b29f0f3016484500a8e91e6a121ed1f18",
    ),
    (
        12,
        "ad4ce1e366518604d42f3cf987059353f91136506578e10a2787a89029a2f06e",
    ),
    (
        13,
        "d89db110f7122fbcbdc82d809ed1322f3b04aaf3e382382214567a176939e825",
    ),
    (
        14,
        "91e615bf3eb276f2708a4abd26e279128c225a2ff021c6ee7bdbe31785ab96f9",
    ),
    (
        15,
        "14f3b7e6ad68327050537701a3f466c5550f0d638e6c493da578edce0e630291",
    ),
    (
        16,
        "313c68556feee4e979c6605f2c5799fc9323eaf0a6363f3ce79195c5130bbfb4",
    ),
    (
        17,
        "b96d7563642ac1db29a39ac9ef9bed6328b57b8fdb686f66efaa8af6ddcca4e3",
    ),
    (
        18,
        "20abf57c19112783d64b925cd8ac7bcc6fff793dc32eb54258c1a3b4589fc0e4",
    ),
    (
        19,
        "7db878213dc831fd27c88597f47ba0cd58de615e0709dafc4928dee55ae8c0ef",
    ),
    (
        20,
        "44a7abdd179830ea382c82e662b53ccce838a00af74b8bd66907a9c14ef1ef68",
    ),
    (
        21,
        "97c3cbcf8fd952b5b56eff58fe7ad6182f00a20040440367922d520610419ab8",
    ),
    (
        22,
        "f6f0fdff964de59b066eabb8a6eacd5ff579966f6f22acf853b3297910ce3501",
    ),
    (
        23,
        "6adb2c1f13983edcf7417b21902f5fef15ff27a9a6cebf74ba5aefa5917230e7",
    ),
    (
        24,
        "74b6603b471c7cb8b5d2942947590c7bc7ef04f7fd08718301116d2e2bb51725",
    ),
    (
        25,
        "b524808bca1be4fc97ab8b72ccc6c1ff69a969507f1c43d073f79d5df4a33e61",
    ),
    (
        26,
        "88095f2d3612efe5f7adbb448cdcf0294c90eff9df01faff9ae6feed1d8e3419",
    ),
    (
        27,
        "4bd60e765dab38b64413407cccad11988356f5db6a51bafcc7e1c07e9f003677",
    ),
    (
        28,
        "2d96ee017910bc51c82d48c1223bd7027f78f3f5846c38b60323e8665a756322",
    ),
    (
        29,
        "226a0b7c01e310496fa2d63daf678ad9b4eb71c37491735148d0af61b7c53524",
    ),
    (
        30,
        "1f214d980857f17dcbf7e74ff0efdb0b3ed775f237d44810d394e1ffd3ce80b7",
    ),
    (
        31,
        "93cebf699d42dbcdedb3270b5798a26e97ff842456b6c23c6246c0780007cfed",
    ),
    (
        32,
        "d95462598b175846a992674e2a1077a6a7b0b3e253414b23600d7baa94e844fd",
    ),
    (
        33,
        "3edd50b97aa29239aacd047c13786ed26514495e046167a8a105693b5d0fef7f",
    ),
    (
        34,
        "405a5caf8234a778397cae009de0ea10ae5b2830960b859a4d6cbc4dc2974ebd",
    ),
    (
        35,
        "57b389e3545b5867aed3b2bac301f96faa5136d978c92c98136e60a32e343fee",
    ),
    (
        36,
        "2c8d0b9bc3e10a06e94bd5ec3a5450c2d76717fa906b5b40e02c0bc9f7caa372",
    ),
    (
        37,
        "0652e173fb192935dc4ab88c8847528bcdbd48dee6a32aeefab092c8e12219a6",
    ),
    (
        38,
        "195ae5cf4817928a421722575ea1b207b280b85161e9eb99c7ed4c1ed6f6d0f6",
    ),
    (
        39,
        "5ab988080e5cbff2b78d74333f1a9a0cdad45c04b8d705c7b6950de0d83457e3",
    ),
    (
        40,
        "3570090725a6aa23c1b863368dcf3c50601ad32ca7b71447dfb1167fb8d81303",
    ),
];

/// The migrations this branch adds, 0041 to 0052. They land together with this table, so they
/// join the ones above from here on: once they are on master they are as fixed as those.
const LANDED_WITH_THIS_BRANCH: [(u32, &str); 12] = [
    (
        41,
        "5fb56462cc1bf3ca45b41b68ba8a03f91e6af456e2dcebdc7e24dd3e95ba8a14",
    ),
    (
        42,
        "496fd2d3fb0a7063596e59b663a1c212bbd6a2682c83368cf36a427f4c64ef18",
    ),
    (
        43,
        "8d4930a7f00360f359fb3670ea9731c996a5d84a2995013336ab9d756e147747",
    ),
    (
        44,
        "9a018114e61ff5e7f6629e205a285a3f8d6d6567956f1ea243b3dd33d422058b",
    ),
    (
        45,
        "f647b7ae08661aef1ff793b0b349deb4b4bed18d91714259ab43c48df963b68c",
    ),
    (
        46,
        "6df02e97167c0a96cc6eeccc21441933b2fa506d970902cef343e9764e15eb7c",
    ),
    (
        47,
        "749ab21acb7ff2c0b17a12e6d2cd72ab3dceef393ed42b99dcd440aeb751530c",
    ),
    (
        48,
        "ad165134b167f90d0b9bf00956a28012ce4e519cb227f4f1031ba552cc6c17e9",
    ),
    (
        49,
        "6f57efecc09d7c29a46071e18d04254c6667edd176a56652c555d31889d8aeaa",
    ),
    (
        50,
        "41e7b528154abe4ca2ca3eb02d1cfecc02c632ce762c76284bad6f81e5942092",
    ),
    (
        51,
        "bfd7f93bc32e9ffb69ab13cf6fbe81e0830ce703c2097af4eab1006ccce56a4b",
    ),
    (
        52,
        "2fabe104464fa993b8c2ca1950d1ecff4d85a068764eb911cefc2c717c6a4ae5",
    ),
];

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A migration that has landed is never edited: a database already past it would never get the
/// change, and the code that reads it would fail there. Add a new migration instead.
#[test]
fn a_migration_that_has_landed_is_not_edited_add_a_new_migration_instead() {
    for (version, want) in LANDED_ON_MASTER.iter().chain(&LANDED_WITH_THIS_BRANCH) {
        let (_, sql) = MIGRATIONS
            .iter()
            .find(|(number, _)| number == version)
            .unwrap_or_else(|| panic!("migration {version:04} was removed"));
        assert_eq!(
            sha256_hex(sql),
            *want,
            "migration {version:04} has landed and was edited; a database past it never runs it \
             again, so add a new migration instead and leave this file as it was"
        );
    }
}

/// Every table's columns as `table.column type notnull default`, sorted.
fn columns_of(db: &Connection) -> Vec<String> {
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|name| name.unwrap())
        .collect();
    let mut columns = Vec::new();
    for table in tables {
        let mut info = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let rows = info
            .query_map([], |row| {
                Ok(format!(
                    "{table}.{} {} {} {:?}",
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?
                ))
            })
            .unwrap();
        columns.extend(rows.map(|row| row.unwrap()));
    }
    columns.sort();
    columns
}

/// The last version on master is 40: a database there, opened by this build, has every table and
/// column a fresh database has, so nothing the code reads is missing from the owner's own.
#[test]
fn a_database_at_the_last_version_on_master_gets_every_column_the_code_reads() {
    let old = Connection::open_in_memory().unwrap();
    apply_upto(&old, 40);
    assert_eq!(version_of(&old), 40);
    migrate::migrate(&old).unwrap();
    assert_eq!(version_of(&old), EXPECTED_VERSION);

    let fresh = Connection::open_in_memory().unwrap();
    migrate::migrate(&fresh).unwrap();
    assert_eq!(columns_of(&old), columns_of(&fresh));
}

/// And the code runs on it: the account's preferences, read and written through the store, on a
/// file that was at version 40 with an account in it.
#[test]
fn the_account_preferences_work_on_a_database_that_was_at_the_last_version_on_master() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        apply_upto(&db, 40);
        seed(&db);
    }
    let store = SqliteStore::open(&path, dir.path()).unwrap();
    assert!(!store.account_prefs(account()).unwrap().keep_labels);
    store
        .set_account_keeps_labels(account(), true, chrono::Utc::now())
        .unwrap();
    assert!(store.account_prefs(account()).unwrap().keep_labels);
}
