//! Upgrading a database that already has data in it.
//!
//! `plan.md` asks for this outright — "a test that opens a checked-in fixture DB from each prior
//! version and migrates it" — under a heading that calls it required from day one. It did not
//! exist. Every other test creates a fresh database, which runs each migration against an
//! *empty* schema, and a migration whose job is to repair data has nothing to repair.
//!
//! That matters most for 0002. Its purpose is to collapse the duplicate `remote_map` rows that
//! every version-1 database accumulated (FINDINGS F52) and then add a unique index. If the
//! `DELETE` misses a single duplicate the `CREATE UNIQUE INDEX` fails, the transaction rolls
//! back, and the application does not start — for exactly the users who have been running it
//! longest.
//!
//! Rather than check in a binary fixture, each version is built by applying the migrations up to
//! it: the SQL is the fixture, and it cannot drift from the migration it describes.

use mail_store::{SqliteStore, migrate};
use rusqlite::Connection;

/// A database at version `upto`, with nothing applied after it.
fn database_at(upto: u32) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    for (version, sql) in migrate::MIGRATIONS.iter().take(upto as usize) {
        db.execute_batch(sql).unwrap();
        if *version > 1 {
            db.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                [version],
            )
            .unwrap();
        }
    }
    db
}

fn version_of(db: &Connection) -> u32 {
    db.query_row(
        "SELECT coalesce(max(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// An account and a message, so `remote_map`'s foreign keys are satisfiable.
fn seed(db: &Connection) -> (String, String) {
    let account = uuid::Uuid::new_v4().to_string();
    let thread = uuid::Uuid::new_v4().to_string();
    let message = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at)
         VALUES (?1, 'me@example.test', '{}', datetime('now'))",
        [&account],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads (id, account, snooze, pin) VALUES (?1, ?2, '\"none\"', '\"none\"')",
        [&thread, &account],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
             recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
             body_text, body_raw, attachments)
         VALUES (?1, ?2, ?3, '\"k\"', '2023-01-01T00:00:00Z', NULL, 'a@b.test', '{}', 's',
             NULL, '[]', NULL, '\"read\"', '\"unstarred\"', '\"inbox\"', NULL, NULL, '[]')",
        [&message, &thread, &account],
    )
    .unwrap();
    (account, message)
}

#[test]
fn a_version_one_database_with_duplicates_upgrades_and_keeps_one_of_each() {
    // The case 0002 exists for, and the one no test had ever produced: a v1 database whose
    // remote_map grew a row per message per sync.
    let db = database_at(1);
    assert_eq!(version_of(&db), 1, "the fixture should start at version 1");
    let (account, message) = seed(&db);

    // Five syncs' worth of the same mapping, which is what v1 produced.
    for _ in 0..5 {
        db.execute(
            "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
             VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
            [&account, &message],
        )
        .unwrap();
    }
    // And a POP-shaped one, which had two NULLs in its key.
    for _ in 0..3 {
        db.execute(
            "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
             VALUES (?1, 'INBOX', NULL, NULL, 'UID-1', ?2)",
            [&account, &message],
        )
        .unwrap();
    }
    let before: i64 = db
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, 8, "the v1 schema really does allow duplicates");

    migrate::migrate(&db).expect("a database with duplicates must still upgrade");

    assert_eq!(version_of(&db), migrate::EXPECTED_VERSION);
    let after: i64 = db
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap();
    // One POP mapping. The IMAP one was collapsed to one by 0002 and then dropped by 0008,
    // which removes every IMAP mapping for the header pass to rebuild.
    assert_eq!(after, 1, "one POP mapping should remain");

    // And the index now prevents what the DELETE just cleaned up.
    db.execute(
        "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
         VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
        [&account, &message],
    )
    .unwrap();
    let again = db.execute(
        "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
         VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
        [&account, &message],
    );
    assert!(
        again.is_err(),
        "after 0002 a duplicate mapping must be refused, not stored again"
    );
}

#[test]
fn every_prior_version_upgrades_to_the_current_one() {
    // Not only v1: each released version must reach the present, because a user upgrades from
    // wherever they happen to be.
    for from in 1..=migrate::EXPECTED_VERSION {
        let db = database_at(from);
        assert_eq!(version_of(&db), from);
        migrate::migrate(&db)
            .unwrap_or_else(|e| panic!("upgrading from version {from} failed: {e}"));
        assert_eq!(
            version_of(&db),
            migrate::EXPECTED_VERSION,
            "version {from} did not reach the current schema"
        );
    }
}

#[test]
fn upgrading_an_already_current_database_changes_nothing() {
    let db = database_at(migrate::EXPECTED_VERSION);
    let (account, message) = seed(&db);
    db.execute(
        "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
         VALUES (?1, 'INBOX', 1, 101, NULL, ?2)",
        [&account, &message],
    )
    .unwrap();

    migrate::migrate(&db).expect("a current database migrates cleanly");
    let rows: i64 = db
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "a no-op migration must not touch data");
}

/// Migration 0004 moved the full-text index off the message columns onto one Rust fills.
///
/// A database written by an older build has no `fts_text`, and SQL cannot compute one — the
/// whole point of the migration is that segmenting a run of ideographs into bigrams is not
/// something a tokenizer or a trigger can do. So `SqliteStore::from_connection` backfills it,
/// and mail that was already on disk has to become findable rather than only new mail.
#[test]
fn mail_that_predates_the_segmented_index_is_findable_afterwards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");

    // A version-3 database with one English and one Chinese message in it.
    {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(3) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, _) = seed(&db);
        let thread: String = db
            .query_row("SELECT id FROM threads LIMIT 1", [], |r| r.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                 recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                 body_text, body_raw, attachments)
             VALUES (?1, ?2, ?3, '\"k2\"', '2023-01-02T00:00:00Z', NULL, 'c@d.test', '{}',
                 '【重要】校園郵件信箱系統維護', NULL, '[]', NULL, '\"read\"', '\"unstarred\"',
                 '\"inbox\"', 'lunch on friday', NULL, '[]')",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), thread, account],
        )
        .unwrap();
    }

    // Opening it upgrades and backfills.
    let store = SqliteStore::open(&path, dir.path()).unwrap();

    // Asked of the index directly rather than through `threads`, which also needs a
    // `thread_summaries` row — this fixture is a hand-built version-3 database, and what the
    // backfill is responsible for is the index.
    let hits = |needle: &str| {
        store
            .connection()
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
                [needle],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(-1)
    };

    assert_eq!(
        hits("lunch"),
        1,
        "English mail already on disk stopped being findable"
    );
    assert_eq!(hits("校園"), 1, "old Chinese mail was not reindexed");
    assert_eq!(hits("維護"), 1);
    assert_eq!(
        hits("臺北"),
        0,
        "and the index did not become a substring match on everything"
    );

    // Idempotent: opening it again does no work and changes nothing.
    drop(store);
    let again = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(
        again
            .connection()
            .query_row(
                "SELECT count(*) FROM messages WHERE fts_text IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

/// Migration 0005 added `To` and `Cc` to the indexed text.
///
/// A version-4 database already has `fts_text`, built without them, so the backfill's `IS NULL`
/// test would skip every row: the migration has to clear the text and the index together. Asked
/// here of a row whose old text is still in the index, because that is the case where getting
/// the order wrong corrupts it — hence the integrity check, and a write after the upgrade to
/// prove the update triggers still agree with what is indexed.
#[test]
fn mail_indexed_before_recipients_were_is_findable_by_them_afterwards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");

    let message = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(4) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, _) = seed(&db);
        let thread: String = db
            .query_row("SELECT id FROM threads LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let message = uuid::Uuid::new_v4().to_string();
        // Indexed the way version 4 did it: subject, sender and body, no recipients.
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
            rusqlite::params![message, thread, account],
        )
        .unwrap();
        message
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let hits = |needle: &str| {
        store
            .connection()
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
                [needle],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(-1)
    };
    let intact = || {
        store
            .connection()
            .execute(
                "INSERT INTO messages_fts(messages_fts) VALUES ('integrity-check')",
                [],
            )
            .map(|_| ())
    };

    assert_eq!(hits("lovelace"), 1, "a To name was not indexed");
    assert_eq!(hits("hopper"), 1, "a Cc address was not indexed");
    assert_eq!(
        hits("lunch"),
        1,
        "what was findable before stopped being findable"
    );
    assert_eq!(
        hits("minutes"),
        1,
        "and the old entry was not left beside the new one"
    );
    assert_eq!(hits("hidden"), 0, "a blind copy must never be searchable");
    intact().expect("the index disagrees with the table after the upgrade");

    store
        .connection()
        .execute(
            "UPDATE messages SET read = '\"unread\"' WHERE id = ?1",
            [&message],
        )
        .unwrap();
    intact().expect("the index disagrees with the table after a write");
    assert_eq!(hits("hopper"), 1);
}

/// Migration 0007: a summary written before embedded images stopped counting is re-derived.
///
/// The stored count is what the list's paperclip and `has:attachment` read, and a thread nobody
/// writes to again would otherwise keep its old count for ever.
#[test]
fn a_summary_that_counted_embedded_images_is_corrected_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(6) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, message) = seed(&db);
        // `seed` writes a key and a thread state no build ever wrote; this test needs a thread
        // that decodes, so it writes real ones.
        db.execute(
            r#"UPDATE messages SET msg_key = '{"kind":"rfc","v":"k@example.test"}' WHERE id = ?1"#,
            [&message],
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
        db.execute(
            r#"UPDATE messages SET attachments = '[{"name":"logo.png","mime":"image/png","size":3,
                "blob":"04040404-0404-0404-0404-040404040404",
                "inline":{"kind":"embedded","v":{"cid":"logo@x"}}}]' WHERE id = ?1"#,
            [&message],
        )
        .unwrap();
        let thread: String = db
            .query_row(
                "SELECT thread FROM messages WHERE id = ?1",
                [&message],
                |r| r.get(0),
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
            rusqlite::params![thread, account],
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let (stored, queued): (String, i64) = store
        .connection()
        .query_row(
            "SELECT (SELECT attachments FROM thread_summary),
                    (SELECT count(*) FROM summaries_to_refresh)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored, r#"{"kind":"none"}"#,
        "the embedded logo is still counted"
    );
    assert_eq!(queued, 0, "the queue is emptied once it has been worked");
}

/// One row that no longer decodes must not stop the database opening after an upgrade.
///
/// Both passes that run on open — the FTS backfill and the summary queue — read every message,
/// so either could lock someone out of all their mail over one bad row. The row keeps what it
/// had; everything else is upgraded.
#[test]
fn an_undecodable_row_does_not_stop_the_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(6) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        // `seed`'s key and recipients are not what any build wrote, so this message decodes
        // neither as a `Message` nor, once its recipients are garbage too, for the index.
        let (account, message) = seed(&db);
        db.execute(
            "UPDATE messages SET recipients = 'not json', fts_text = NULL WHERE id = ?1",
            [&message],
        )
        .unwrap();
        let thread: String = db
            .query_row(
                "SELECT thread FROM messages WHERE id = ?1",
                [&message],
                |r| r.get(0),
            )
            .unwrap();
        db.execute(
            r#"INSERT INTO thread_summary (thread, account, subject, snippet, from_name,
                 from_email, participants, recipients, last_date, message_count, read, star,
                 mailboxes, labels, attachments, snooze, pin)
               VALUES (?1, ?2, 's', '', NULL, 'a@b.test', '[]', '[]',
                 '2023-01-01T00:00:00.000000000Z', 1, '"read"', '"unstarred"', '["inbox"]',
                 '[]', '{"kind":"none"}', '"none"', '"none"')"#,
            rusqlite::params![thread, account],
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path, dir.path()).expect("one bad row must not lock anyone out");
    assert_eq!(version_of(&store.connection()), migrate::EXPECTED_VERSION);
}

/// 0008: every IMAP mapping goes, because any of them may name the wrong message; POP3's stay.
#[test]
fn imap_mappings_are_dropped_so_the_header_pass_can_rebuild_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let message = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(7) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, message) = seed(&db);
        db.execute(
            "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
             VALUES (?1, 'INBOX', 1, 42, NULL, ?2), (?1, 'INBOX', NULL, NULL, 'u1', ?2)",
            [&account, &message],
        )
        .unwrap();
        message
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let left: Vec<(Option<i64>, Option<String>)> = store
        .connection()
        .prepare("SELECT uid, uidl FROM remote_map")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(left, [(None, Some("u1".to_owned()))]);
    let kept: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM messages WHERE id = ?1",
            [&message],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1, "the message itself stays");
}

/// 0009: an account that predates folder listings upgrades with none, keeps its mail, and can
/// be given a listing straight away.
#[test]
fn an_account_from_before_folders_upgrades_with_an_empty_listing() {
    use mail_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (account, message) = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(8) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        seed(&db)
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let account = mail_domain::AccountId::from_uuid(account.parse().unwrap());
    assert_eq!(store.folders(account).unwrap(), vec![]);
    let inbox = mail_domain::Folder {
        account,
        path: "INBOX".to_owned(),
        delimiter: Some('/'),
        special: Some(mail_domain::SpecialUse::Inbox),
        subscription: mail_domain::Subscription::Subscribed,
        holds: mail_domain::Holds::Mail,
    };
    store.put_folders(account, vec![inbox.clone()]).unwrap();
    assert_eq!(store.folders(account).unwrap(), vec![inbox]);
    let kept: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM messages WHERE id = ?1",
            [&message],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1);
}

/// 0010: a database from before notifications has no account armed, so the first watched pass
/// after the upgrade arms each one — and deleting an account takes its floor with it.
#[test]
fn accounts_from_before_notifications_start_unarmed() {
    let db = database_at(9);
    let (account, _message) = seed(&db);
    migrate::migrate(&db).unwrap();
    assert_eq!(version_of(&db), migrate::EXPECTED_VERSION);

    let armed: i64 = db
        .query_row("SELECT count(*) FROM notify_floor", [], |r| r.get(0))
        .unwrap();
    assert_eq!(armed, 0, "nothing is armed until something watches");

    db.execute(
        "INSERT INTO notify_floor (account, armed_at) VALUES (?1, '2026-09-24T00:00:00+00:00')",
        [&account],
    )
    .unwrap();
    db.execute("DELETE FROM accounts WHERE id = ?1", [&account])
        .unwrap();
    let left: i64 = db
        .query_row("SELECT count(*) FROM notify_floor", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "a removed account's floor goes with it");
}

/// 0011: a draft saved before receipts existed opens as one that asks for none, and the answers
/// table starts empty and goes with its message.
#[test]
fn drafts_from_before_receipts_load_without_asking_for_one() {
    use mail_domain::{DraftId, ReceiptAnswer, ReceiptRequest};
    use mail_store::Store;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (draft, message) = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(10) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, message) = seed(&db);
        let identity = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [&identity, &account],
        )
        .unwrap();
        let draft = uuid::Uuid::new_v4();
        db.execute(
            r#"INSERT INTO drafts (id, account, identity, recipients, subject, in_reply_to,
                 forward_of, body_text, body_html, attachments, state, updated_at)
               VALUES (?1, ?2, ?3, '{"reply_to":[],"to":[],"cc":[],"bcc":[]}', 'old', NULL,
                 NULL, 'text', NULL, '[]', '{"kind":"editing"}',
                 '2023-01-01T00:00:00.000000000Z')"#,
            rusqlite::params![draft.to_string(), account, identity],
        )
        .unwrap();
        (DraftId::from_uuid(draft), message)
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(version_of(&store.connection()), migrate::EXPECTED_VERSION);
    let loaded = store.draft(draft).expect("the old draft still loads");
    assert_eq!(loaded.subject, "old");
    assert_eq!(loaded.receipt, ReceiptRequest::Unrequested);

    let message = mail_domain::MessageId::from_uuid(message.parse().unwrap());
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    assert_eq!(store.receipt_answer(message).unwrap(), None);
    store
        .answer_receipt(message, ReceiptAnswer::Declined, at)
        .unwrap();
    store
        .connection()
        .execute("DELETE FROM messages", [])
        .unwrap();
    let left: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM receipt_answers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "an answer does not outlive its message");
}

/// Mail held before the address book existed is read into it once, on the first open after the
/// upgrade — a sent draft as written to, and the Sent-folder copy of it not counted again.
#[test]
fn mail_held_before_the_address_book_fills_it_on_open() {
    use mail_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(12) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        // `seed`'s message is from a@b.test, in the inbox. Add the user's identity, a copy in
        // Sent, and the sent draft it is a copy of.
        let (account, _inbox) = seed(&db);
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES ('00000000-0000-4000-8000-0000000000b1', ?1, 'Me', 'me@example.test',
                     '\"default\"')",
            [&account],
        )
        .unwrap();
        let thread: String = db
            .query_row("SELECT id FROM threads", [], |r| r.get(0))
            .unwrap();
        db.execute(
            r#"INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                   recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                   body_text, body_raw, attachments)
               VALUES ('00000000-0000-4000-8000-0000000000c1', ?1, ?2, '"k2"',
                   '2023-01-02T00:00:00.000000000Z', 'Me', 'me@example.test',
                   '{"to":[{"name":"Bob","email":"bob@example.test"}]}', 'Plans', NULL, '[]',
                   NULL, '"read"', '"unstarred"', '"sent"', NULL, NULL, '[]')"#,
            [&thread, &account],
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
            [&account, &state],
        )
        .unwrap();
    }

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let book = store.contacts().unwrap();
    let find = |address: &str| book.iter().find(|c| c.address == address).cloned();
    assert_eq!(find("a@b.test").map(|c| c.received.count), Some(1));
    let bob = find("bob@example.test").unwrap();
    assert_eq!(
        bob.written.count, 1,
        "the draft and its Sent copy are one message"
    );
    assert_eq!(bob.name.as_deref(), Some("Bob Typed"));
    assert_eq!(
        find("me@example.test").map(|c| c.kind),
        Some(mail_store::Kind::Own)
    );
    let pending: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM contacts_to_backfill", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(pending, 0, "the backfill runs once");
    drop(store);

    let again = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(
        again.contacts().unwrap(),
        book,
        "a second open counts nothing twice"
    );
}

/// 0014: a database from before templates upgrades with none, keeps its drafts, and can keep a
/// template straight away.
#[test]
fn a_database_from_before_templates_upgrades_with_none_and_keeps_its_drafts() {
    use mail_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (account, identity, draft) = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(13) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, _) = seed(&db);
        let identity = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            [&identity, &account],
        )
        .unwrap();
        // A draft row as every build before this one wrote it, with a state they knew.
        let draft = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO drafts (id, account, identity, recipients, subject, body_text,
                 attachments, state, updated_at)
             VALUES (?1, ?2, ?3, '{\"to\":[{\"name\":null,\"email\":\"you@example.test\"}]}',
                 'kept', 'hello', '[]', '{\"kind\":\"queued\"}', '2026-09-01T00:00:00Z')",
            [&draft, &account, &identity],
        )
        .unwrap();
        (account, identity, draft)
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let account = mail_domain::AccountId::from_uuid(account.parse().unwrap());
    assert_eq!(store.templates(account).unwrap(), vec![]);
    let old = store
        .draft(mail_domain::DraftId::from_uuid(draft.parse().unwrap()))
        .unwrap();
    assert_eq!(old.state, mail_domain::SendState::Queued);

    let kept = mail_domain::Template::from_draft(&old, "", old.updated);
    assert_eq!(kept.identity.to_string(), identity);
    store.put_template(&kept).unwrap();
    assert_eq!(store.templates(account).unwrap(), vec![kept]);
}

/// 0015: what body fetches drawn from two mailboxes at once left behind is undone, and nothing
/// else is touched.
#[test]
fn addresses_and_bodies_a_mixed_body_batch_damaged_are_cleared_for_refetching() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(14) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, _) = seed(&db);
        let thread: String = db
            .query_row("SELECT id FROM threads", [], |r| r.get(0))
            .unwrap();
        db.execute(
            "INSERT INTO blobs (id, hash, size, path, inline, created_at)
             VALUES ('b', 'h', 1, NULL, x'00', datetime('now'))",
            [],
        )
        .unwrap();
        let message = |id: &str, key: &str, subject: &str, body: Option<&str>| {
            db.execute(
                "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                     body_text, body_raw, attachments)
                 VALUES (?1, ?2, ?3, ?4, '2023-01-02T00:00:00Z', NULL, 'a@b.test', '{}', ?5,
                     NULL, '[]', NULL, '\"read\"', '\"unstarred\"', '\"inbox\"', ?6,
                     CASE WHEN ?6 IS NULL THEN NULL ELSE 'b' END, '[]')",
                rusqlite::params![id, thread, account, key, subject, body],
            )
            .unwrap();
        };
        let at = |mailbox: &str, uid: i64, message: &str| {
            db.execute(
                "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, uidl, message)
                 VALUES (?1, ?2, 1, ?3, NULL, ?4)",
                rusqlite::params![account, mailbox, uid, message],
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
        message(
            "n",
            "{\"kind\":\"synthetic\",\"v\":[1]}",
            "no id",
            Some("hi"),
        );
        message(
            "copy",
            "{\"kind\":\"synthetic\",\"v\":[9]}",
            "no id",
            Some("hi"),
        );
        at("INBOX", 6, "n");
        at("Sent", 6, "copy");
        // A large message rebuilt around another's structure.
        message(
            "big",
            &rfc("big"),
            "large",
            Some("--other\r\nContent-Type: application/pdf\r\nX-Mailo-Remote-Section: 2\r\n"),
        );
        at("Sent", 9, "big");
        // One held in two mailboxes under two UIDs, as a server keeps copies: untouched.
        message("both", &rfc("both"), "copied", Some("-- \r\nsignature"));
        at("INBOX", 7, "both");
        at("Projects", 8, "both");
    }

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let db = store.connection();
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
    // Which of two identical digest-keyed messages is the copy cannot be told from here, so
    // both go; the header pass stores the one the server really holds again.
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
}

/// 0016: a database from before invitation answers upgrades with none; an answer can be kept
/// straight away, replaced, and goes with its message.
#[test]
fn a_database_from_before_invitation_answers_upgrades_and_keeps_them_per_message() {
    use mail_domain::{Attendance, InviteAnswer, MessageId};
    use mail_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let message = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(15) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (_, message) = seed(&db);
        message
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(version_of(&store.connection()), migrate::EXPECTED_VERSION);
    let message = MessageId::from_uuid(message.parse().unwrap());
    assert_eq!(store.invite_answer(message).unwrap(), None);
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let answer = InviteAnswer {
        message,
        attendance: Attendance::Accepted,
        sequence: 0,
        comment: None,
        answered_at: at,
    };
    store.answer_invite(&answer).unwrap();
    let changed = InviteAnswer {
        attendance: Attendance::Declined,
        comment: Some("clash".to_owned()),
        ..answer
    };
    store.answer_invite(&changed).unwrap();
    assert_eq!(store.invite_answer(message).unwrap(), Some(changed));
    store
        .connection()
        .execute("DELETE FROM messages", [])
        .unwrap();
    let left: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM invite_answers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "an answer does not outlive its message");
}

/// 0017: a database from before rules upgrades with none and no vacation reply, keeps its mail,
/// and can keep a rule straight away.
#[test]
fn a_database_from_before_rules_upgrades_with_none_and_keeps_its_mail() {
    use mail_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let (account, message) = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(16) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        seed(&db)
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    let account = mail_domain::AccountId::from_uuid(account.parse().unwrap());
    assert_eq!(store.rules(account).unwrap(), vec![]);
    assert_eq!(store.vacation(account).unwrap(), None);
    let kept: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM messages WHERE id = ?1",
            [&message],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1, "the mail is still there");

    let rule = mail_domain::Rule {
        id: mail_domain::RuleId::generate(),
        account,
        name: "Bills".to_owned(),
        position: 1,
        state: mail_domain::RuleState::Enabled,
        filter: mail_domain::Filter::All,
        actions: vec![mail_domain::RuleAction::Archive],
        after: mail_domain::AfterMatch::Continue,
    };
    store.put_rule(&rule).unwrap();
    assert_eq!(store.rules(account).unwrap(), vec![rule]);
}

/// 0018: a draft saved before OpenPGP opens as plain, and the key and Autocrypt tables start
/// empty and take a key straight away.
#[test]
fn a_database_from_before_openpgp_upgrades_with_plain_drafts_and_no_keys() {
    use mail_domain::{DraftId, Fingerprint, KeySource, KeyTrust, OpenPgp, PgpKey, SecretHeld};
    use mail_store::Store;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let draft = {
        let db = Connection::open(&path).unwrap();
        for (version, sql) in migrate::MIGRATIONS.iter().take(17) {
            db.execute_batch(sql).unwrap();
            if *version > 1 {
                db.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                    [version],
                )
                .unwrap();
            }
        }
        let (account, _) = seed(&db);
        let identity = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [&identity, &account],
        )
        .unwrap();
        let draft = uuid::Uuid::new_v4();
        db.execute(
            r#"INSERT INTO drafts (id, account, identity, recipients, subject, in_reply_to,
                 forward_of, body_text, body_html, attachments, state, updated_at, receipt)
               VALUES (?1, ?2, ?3, '{"reply_to":[],"to":[],"cc":[],"bcc":[]}', 'old', NULL,
                 NULL, 'text', NULL, '[]', '{"kind":"editing"}',
                 '2023-01-01T00:00:00.000000000Z', '"unrequested"')"#,
            rusqlite::params![draft.to_string(), account, identity],
        )
        .unwrap();
        DraftId::from_uuid(draft)
    };

    let store = SqliteStore::open(&path, dir.path()).unwrap();
    assert_eq!(version_of(&store.connection()), migrate::EXPECTED_VERSION);
    assert_eq!(store.draft(draft).unwrap().openpgp, OpenPgp::None);
    assert!(store.pgp_keys().unwrap().is_empty());
    assert_eq!(store.autocrypt_peer("me@example.test").unwrap(), None);
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let key = PgpKey {
        fingerprint: Fingerprint::V4([3; 20]),
        key_ids: vec![Fingerprint::V4([3; 20]).key_id()],
        user_ids: vec!["Me <me@example.test>".to_owned()],
        emails: vec!["me@example.test".to_owned()],
        key: vec![0x98, 0x33],
        source: KeySource::Imported,
        first_seen: at,
        last_seen: at,
        trust: KeyTrust::Unverified,
        secret: SecretHeld::Absent,
    };
    store.put_pgp_key(key.clone()).unwrap();
    assert_eq!(store.pgp_keys_for("me@example.test").unwrap(), vec![key]);
}
