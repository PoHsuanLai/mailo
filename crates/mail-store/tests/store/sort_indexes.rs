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
    // The shape `threads` builds: its sort column, then the date and the thread as the
    // tie-breaks, all in the one direction. Date is 0003's and orders by the thread alone after
    // itself; the others are 0033's.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let orders = std::iter::once("ts.last_date, ts.thread".to_owned()).chain(
        [
            "sort_subject",
            "sort_sender",
            "size",
            "unread",
            "starred",
            "pin",
        ]
        .map(|column| format!("ts.{column}, ts.last_date, ts.thread")),
    );
    for order in orders {
        for dir in ["ASC", "DESC"] {
            let by = order
                .split(", ")
                .map(|column| format!("{column} {dir}"))
                .collect::<Vec<_>>()
                .join(", ");
            let plan = plan(
                &store,
                &format!(
                    "SELECT ts.thread FROM thread_summary ts WHERE 1=1 ORDER BY {by} LIMIT 51"
                ),
            );
            assert!(!plan.contains("TEMP B-TREE"), "{by}: {plan}");
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
