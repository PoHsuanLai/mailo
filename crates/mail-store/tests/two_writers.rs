//! Two connections writing the same database, which is an ordinary thing to do.
//!
//! `mailo sync` in a terminal while the window is open, or a scheduled sync overlapping a manual
//! one. WAL lets a reader run beside a writer and does nothing for two writers: SQLite serialises
//! them, and the second either waits or is told "database is locked".

use mail_domain::*;
use mail_store::SqliteStore;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn opened(dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir).unwrap();
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    store
}

fn pragma(store: &SqliteStore, name: &str) -> String {
    store
        .connection()
        .query_row(&format!("PRAGMA {name}"), [], |r| {
            r.get::<_, rusqlite::types::Value>(0)
        })
        .map(|v| match v {
            rusqlite::types::Value::Integer(i) => i.to_string(),
            rusqlite::types::Value::Text(t) => t,
            other => format!("{other:?}"),
        })
        .unwrap()
}

#[test]
fn a_contended_write_waits_rather_than_failing_at_once() {
    // The default that decides this is `rusqlite`'s, not ours, and a default nothing names is a
    // behaviour nobody notices changing. Asserted on a real file and on an in-memory database,
    // because they are opened by different code paths.
    let dir = tempfile::tempdir().unwrap();
    for store in [
        opened(dir.path()),
        SqliteStore::in_memory(dir.path()).unwrap(),
    ] {
        assert_eq!(pragma(&store, "busy_timeout"), "5000");
    }
}

#[test]
fn the_pragmas_that_keep_the_window_responsive_are_set() {
    // WAL is what lets the list redraw while a sync commits. A database that fell back to
    // `delete` journalling would block every read behind every write, which is the shape of
    // "the window freezes while it syncs".
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    assert_eq!(pragma(&store, "journal_mode"), "wal");
    assert_eq!(pragma(&store, "synchronous"), "1", "NORMAL");
}

#[test]
#[ignore = "takes the full timeout on purpose"]
fn a_write_held_open_makes_the_other_wait_the_timeout_and_then_say_so() {
    // What the number actually buys, demonstrated rather than asserted: five seconds of waiting
    // and then a plain "database is locked" rather than an immediate one. Ignored because the
    // demonstration is the five seconds.
    let dir = tempfile::tempdir().unwrap();
    let a = opened(dir.path());
    let b = opened(dir.path());

    let held = a.connection();
    held.execute_batch("BEGIN IMMEDIATE").unwrap();
    held.execute(
        "INSERT INTO labels (id, account, name, color, origin)
         VALUES (?1, ?2, 'held', NULL, '\"provider\"')",
        rusqlite::params![LabelId::generate().to_string(), ACCOUNT.to_string()],
    )
    .unwrap();

    let started = std::time::Instant::now();
    let outcome = b.connection().execute(
        "INSERT INTO labels (id, account, name, color, origin)
         VALUES (?1, ?2, 'other', NULL, '\"provider\"')",
        rusqlite::params![LabelId::generate().to_string(), ACCOUNT.to_string()],
    );
    let waited = started.elapsed();
    held.execute_batch("COMMIT").unwrap();

    assert!(outcome.is_err(), "it should give up rather than hang");
    assert!(
        waited >= std::time::Duration::from_secs(4),
        "it gave up after {waited:?}, so the timeout is not what it says"
    );
}
