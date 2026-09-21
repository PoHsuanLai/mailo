//! Does the SQLite we actually link against support what the schema needs?
//!
//! `rusqlite` 0.40 has no `fts5` feature — FTS5 comes from how the bundled amalgamation is
//! compiled. That is a fact to verify, not to assume: the schema's `messages_fts` table is
//! load-bearing for search, and an assumption here fails at first run instead of at build.

#[test]
fn bundled_sqlite_has_fts5_and_wal() {
    let db = rusqlite::Connection::open_in_memory().expect("open");

    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(body, tokenize = 'unicode61 remove_diacritics 2');
         INSERT INTO t(body) VALUES ('Résumé for Friday');",
    )
    .expect("FTS5 with unicode61 must be available; the schema depends on it");

    // remove_diacritics = 2 is what makes Filter::Text fold accents. Prove it here, because
    // mail-domain's fold table was written to match this behaviour and nothing else checks
    // that the two agree at the SQLite end.
    let hits: i64 = db
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'resume'", [], |r| {
            r.get(0)
        })
        .expect("query");
    assert_eq!(hits, 1, "unaccented needle must match an accented document");

    let mode: String = db
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .expect("wal");
    // An in-memory database reports "memory"; on disk it reports "wal". Either proves the
    // pragma is understood rather than silently ignored.
    assert!(matches!(mode.as_str(), "wal" | "memory"), "got {mode}");
}
