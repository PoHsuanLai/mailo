//! The orders the store reads in come out of an index, not a sort of every row.
//!
//! A temp B-tree over the mailbox costs nothing at three rows and is a pause on every page at
//! ten thousand, so what is asserted is the plan rather than a timing.

use mail_store::SqliteStore;

fn plan(store: &SqliteStore, sql: &str) -> String {
    let db = store.raw_connection();
    let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.join("\n")
}

#[test]
fn every_order_of_the_list_is_read_from_an_index() {
    // The shape `threads` builds: its sort column, then the thread as the tie-break, both in the
    // one direction. Date is 0003's; subject, sender and pin are 0032's.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for column in ["last_date", "subject", "from_email", "pin"] {
        for dir in ["ASC", "DESC"] {
            let plan = plan(
                &store,
                &format!(
                    "SELECT ts.thread FROM thread_summary ts WHERE 1=1 \
                     ORDER BY ts.{column} {dir}, ts.thread {dir} LIMIT 51"
                ),
            );
            assert!(!plan.contains("TEMP B-TREE"), "{column} {dir}: {plan}");
        }
    }
}

#[test]
fn a_conversation_is_read_in_order_from_an_index() {
    // Every open and every summary rebuild reads a thread's messages oldest first.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let plan = plan(
        &store,
        "SELECT id FROM messages WHERE thread = 'x' ORDER BY date, id",
    );
    assert!(plan.contains("messages_thread_order"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}
