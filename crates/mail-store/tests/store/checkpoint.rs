//! The write-ahead log is capped, and folded back when asked.
//!
//! Without a size limit the `-wal` file stays as large as it ever got; these tests are on a store
//! in a temporary directory, never the real one.

use mail_store::{Checkpoint, SqliteStore};

fn opened(dir: &std::path::Path) -> SqliteStore {
    SqliteStore::open(dir.join("mail.db"), dir).unwrap()
}

/// Write enough rows, in several transactions, for the log to hold well over a megabyte.
fn grow_log(store: &SqliteStore) {
    let db = store.raw_connection();
    db.execute_batch("CREATE TABLE IF NOT EXISTS filler (body BLOB)")
        .unwrap();
    for _ in 0..20 {
        db.execute("INSERT INTO filler (body) VALUES (zeroblob(200000))", [])
            .unwrap();
    }
}

fn log_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("mail.db-wal")
}

#[test]
fn the_writer_caps_how_large_the_log_may_stay() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let limit: i64 = store
        .raw_connection()
        .query_row("PRAGMA journal_size_limit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(limit, 67_108_864);
}

#[test]
fn a_truncating_checkpoint_empties_the_log_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    grow_log(&store);
    let grown = std::fs::metadata(log_path(dir.path())).unwrap().len();
    assert!(
        grown > 1_000_000,
        "the log should have grown, it is {grown}"
    );
    assert_eq!(store.log_size(), grown);

    store.checkpoint(Checkpoint::Truncate).unwrap();

    let left = std::fs::metadata(log_path(dir.path())).map_or(0, |m| m.len());
    assert_eq!(left, 0);
    assert_eq!(store.log_size(), 0);
}

#[test]
fn a_passive_checkpoint_returns_while_a_reader_holds_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    grow_log(&store);

    // Another connection stands in for a reader mid-read: its snapshot is what keeps the log from
    // being folded back past it.
    let reader = rusqlite::Connection::open(dir.path().join("mail.db")).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    reader
        .query_row("SELECT count(*) FROM filler", [], |r| r.get::<_, i64>(0))
        .unwrap();
    grow_log(&store);

    store.checkpoint(Checkpoint::Passive).unwrap();

    reader.execute_batch("COMMIT").unwrap();
}
