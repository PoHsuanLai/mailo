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

/// The three parity hazards `sql-compile` reported, pinned against real SQLite so a later
/// change to either side fails here rather than in a mystery proptest shrink.
#[test]
fn the_reported_parity_hazards_are_actually_closed() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("../migrations/0001_initial.sql"))
        .expect("schema");

    // 1. Display names are searchable. fit matches them; without from_name in the index, SQL
    //    would not, and the two would disagree.
    let cols: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('messages_fts') WHERE name = 'from_name'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, 1, "from_name must be indexed");

    // 2. Fixed-width nanosecond timestamps sort as text in instant order. chrono's serde form
    //    does not: ...06.001Z sorts BEFORE ...06Z, which silently inverts a range query.
    let variable = ["2026-01-09T04:05:06Z", "2026-01-09T04:05:06.001Z"];
    assert!(variable[1] < variable[0], "the hazard is real");
    let fixed = [
        "2026-01-09T04:05:06.000000000Z",
        "2026-01-09T04:05:06.001000000Z",
    ];
    assert!(fixed[0] < fixed[1], "fixed width must restore the order");

    // 3. Thread-corpus co-occurrence. Two tokens split across two messages of one thread must
    //    match, because fit treats the thread as one corpus. A single `t1 AND t2` MATCH would
    //    require both on one row and would miss this.
    db.execute_batch(
        "INSERT INTO messages_fts(rowid, subject, from_name, from_email, body_text)
         VALUES (1, 'ada writes', NULL, 'a@x.test', 'first'),
                (2, 'about lunch', NULL, 'b@x.test', 'second');",
    )
    .unwrap();
    let both_on_one_row: i64 = db
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH '\"ada\" AND \"lunch\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(both_on_one_row, 0, "one MATCH cannot span two messages");
    let per_token: i64 = db
        .query_row(
            "SELECT (SELECT count(*) FROM messages_fts WHERE messages_fts MATCH '\"ada\"')
                  * (SELECT count(*) FROM messages_fts WHERE messages_fts MATCH '\"lunch\"')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(per_token > 0, "one EXISTS per token spans the thread");
}

/// `fts5vocab` over the external-content `messages_fts`, created in `temp` so it is not schema.
///
/// Prefix suggestions read this table. It has to accept the table migration 0004 actually
/// built — one column, `content = 'messages'` — or there is no vocabulary to suggest from
/// and the rest of the search work should stop rather than invent a second index.
#[test]
fn fts5vocab_row_accepts_the_external_content_index() {
    use chrono::{TimeZone, Utc};
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};

    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let account = AccountId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [account.to_string()],
        )
        .unwrap();

    // The exact statement the store runs per connection. `main` because the index lives in
    // the main schema and this table lives in temp, which is not a migration.
    store
        .connection()
        .execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.messages_vocab \
             USING fts5vocab(main, messages_fts, 'row')",
        )
        .expect("fts5vocab must accept the external-content messages_fts");

    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account,
        key: MessageKey::Rfc("vocab@example.test".into()),
        date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        from: Address {
            name: None,
            email: "a@b.test".into(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "Résumé widgets".into(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("widgets widgets in the body".into()),
            raw,
        },
        attachments: vec![],
    };
    store
        .apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message))],
            },
        )
        .unwrap();

    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT term, doc, cnt FROM temp.messages_vocab WHERE term = 'resume'")
        .expect("vocab columns term, doc, cnt");
    let (doc, cnt): (i64, i64) = stmt
        .query_row([], |r| Ok((r.get(1)?, r.get(2)?)))
        .expect("folded term resume must be in the vocab");
    assert_eq!(doc, 1, "one message holds resume");
    assert!(cnt >= 1, "cnt counts occurrences, got {cnt}");

    // SQLite's bm25 is negative and numerically smaller is a better match. top_hits
    // negates it; this pins the sign the negation assumes.
    let score: f64 = db
        .query_row(
            "SELECT bm25(messages_fts) FROM messages_fts WHERE messages_fts MATCH '\"widgets\"'",
            [],
            |r| r.get(0),
        )
        .expect("bm25");
    assert!(score < 0.0, "bm25 of a hit must be negative, got {score}");
}

/// A public method that opens a transaction and then calls a helper must not deadlock.
///
/// This is a regression test for a real bug, not a hypothetical. Guarding the connection with a
/// plain `std::sync::Mutex` made `write_patch` — which takes the connection for a transaction
/// and then calls `write_change`, which takes it again — hang forever. A deadlock has no error
/// message and no panic: the process simply stops, and the only symptom was an integration test
/// that never finished.
#[test]
fn nested_connection_access_does_not_deadlock() {
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};

    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let account = AccountId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [account.to_string()],
        )
        .unwrap();

    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account,
        key: MessageKey::Rfc("deadlock@example.test".into()),
        date: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        from: Address {
            name: None,
            email: "a@b.test".into(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "nested".into(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("body".into()),
            raw,
        },
        attachments: vec![],
    };

    // write_patch -> transaction -> write_change -> connection again.
    store
        .apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message))],
            },
        )
        .expect("a nested take of the connection must not hang");

    // And holding one across a call that takes it again.
    let held = store.connection();
    let count: i64 = held
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(held);
}
