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

use mail_store::migrate;
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
    assert_eq!(
        after, 2,
        "one IMAP mapping and one POP mapping should remain"
    );

    // And the index now prevents what the DELETE just cleaned up.
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
