//! Which mailboxes and labels a thread is in, as the tables the lists search hold them.
//!
//! `thread_mailbox` and `thread_label` repeat what the summary's JSON says so that an index can
//! find a mailbox's threads. They are only worth having if they never disagree with it.

use crate::support::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

fn members(store: &SqliteStore, table: &str, column: &str, thread: ThreadId) -> Vec<String> {
    let db = store.raw_connection();
    db.prepare(&format!(
        "SELECT {column} FROM {table} WHERE thread = ?1 ORDER BY {column}"
    ))
    .unwrap()
    .query_map([thread.to_string()], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn mailboxes_of(store: &SqliteStore, thread: ThreadId) -> Vec<String> {
    members(store, "thread_mailbox", "mailbox", thread)
}

fn labels_of(store: &SqliteStore, thread: ThreadId) -> Vec<String> {
    members(store, "thread_label", "label", thread)
}

fn delivered(store: &SqliteStore, n: u128, mailbox: MailboxRole) -> Message {
    let m = Message {
        mailbox,
        ..message(n)
    };
    deliver(store, &m, imap("INBOX", n as u32));
    m
}

fn label(store: &SqliteStore, name: &str) -> Label {
    let label = Label {
        id: LabelId::generate(),
        account: acct_account(),
        name: name.to_owned(),
        color: None,
        origin: LabelOrigin::User,
    };
    store
        .apply(
            acct_account(),
            &patch(vec![Change::LabelUpsert(label.clone())]),
        )
        .unwrap();
    label
}

#[test]
fn a_thread_is_in_the_mailbox_its_message_arrived_in() {
    let store = sqlite();
    let m = delivered(&store, 1, MailboxRole::Inbox);
    assert_eq!(mailboxes_of(&store, m.thread), ["inbox"]);
}

#[test]
fn a_thread_with_messages_in_two_mailboxes_is_in_both() {
    let store = sqlite();
    let first = delivered(&store, 1, MailboxRole::Inbox);
    let reply = Message {
        thread: first.thread,
        mailbox: MailboxRole::Sent,
        ..message(2)
    };
    deliver(&store, &reply, imap("Sent", 2));
    assert_eq!(mailboxes_of(&store, first.thread), ["inbox", "sent"]);
}

#[test]
fn a_message_that_moves_takes_its_thread_with_it() {
    let store = sqlite();
    let m = delivered(&store, 1, MailboxRole::Inbox);
    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageMailbox(m.id, MailboxRole::Archive)]),
        )
        .unwrap();
    assert_eq!(mailboxes_of(&store, m.thread), ["archive"]);
}

#[test]
fn a_label_gained_and_lost_comes_and_goes() {
    let store = sqlite();
    let m = delivered(&store, 1, MailboxRole::Inbox);
    let work = label(&store, "work");
    assert!(labels_of(&store, m.thread).is_empty());

    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageLabel(m.id, work.id, Membership::In)]),
        )
        .unwrap();
    assert_eq!(labels_of(&store, m.thread), [work.id.to_string()]);

    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageLabel(m.id, work.id, Membership::Out)]),
        )
        .unwrap();
    assert!(labels_of(&store, m.thread).is_empty());
}

#[test]
fn a_thread_that_is_deleted_leaves_no_rows_behind() {
    let store = sqlite();
    let m = delivered(&store, 1, MailboxRole::Inbox);
    let work = label(&store, "work");
    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageLabel(m.id, work.id, Membership::In)]),
        )
        .unwrap();
    store
        .apply(acct_account(), &patch(vec![Change::MessageDelete(m.id)]))
        .unwrap();

    assert!(mailboxes_of(&store, m.thread).is_empty());
    assert!(labels_of(&store, m.thread).is_empty());
}

#[test]
fn a_summary_that_changed_only_in_its_read_state_leaves_its_rows_alone() {
    // The rows are rewritten only when the set changed, as the summary itself is only written
    // when something in it did. A trigger counts the rows put in from here on.
    let store = sqlite();
    let m = delivered(&store, 1, MailboxRole::Inbox);
    store
        .raw_connection()
        .execute_batch(
            "CREATE TEMP TABLE put_in (thread TEXT);
             CREATE TEMP TRIGGER counted AFTER INSERT ON main.thread_mailbox
                 BEGIN INSERT INTO put_in VALUES (new.thread); END;",
        )
        .unwrap();
    let put_in = || -> i64 {
        store
            .raw_connection()
            .query_row("SELECT count(*) FROM put_in", [], |r| r.get(0))
            .unwrap()
    };

    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageRead(m.id, ReadState::Read)]),
        )
        .unwrap();
    assert_eq!(put_in(), 0, "a read flag rewrote the membership rows");

    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageMailbox(m.id, MailboxRole::Archive)]),
        )
        .unwrap();
    assert_eq!(put_in(), 1, "a move did not write the new row");
}

/// The threads the summaries' own JSON puts in `role`, newest first: what a list of it was before
/// there were membership tables.
fn by_json(store: &SqliteStore, clause: &str, value: &str) -> Vec<String> {
    let db = store.raw_connection();
    db.prepare(&format!(
        "SELECT ts.thread FROM thread_summary ts WHERE {clause} \
         ORDER BY ts.last_date DESC, ts.thread DESC"
    ))
    .unwrap()
    .query_map([value], |r| r.get(0))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn lists_and_counts_are_what_the_summaries_json_says_on_a_mixed_store() {
    let store = sqlite();
    let work = label(&store, "work");
    let home = label(&store, "home");
    let roles = MailboxRole::ALL;
    for n in 1..=60u128 {
        let m = delivered(&store, n, roles[(n % 6) as usize]);
        let mut changes = Vec::new();
        if n % 4 == 0 {
            changes.push(Change::MessageLabel(m.id, work.id, Membership::In));
        }
        if n % 10 == 0 {
            changes.push(Change::MessageLabel(m.id, home.id, Membership::In));
        }
        if n % 7 == 0 {
            changes.push(Change::MessageMailbox(m.id, MailboxRole::Archive));
        }
        if !changes.is_empty() {
            store.apply(acct_account(), &patch(changes)).unwrap();
        }
    }

    let listed = |filter: Filter| -> Vec<String> {
        store
            .threads(&query(filter, 100), at(0))
            .unwrap()
            .items
            .into_iter()
            .map(|s| s.id.to_string())
            .collect()
    };
    for role in roles {
        let want = by_json(
            &store,
            "EXISTS (SELECT 1 FROM json_each(ts.mailboxes) WHERE value = ?1)",
            role.as_str(),
        );
        assert!(
            !want.is_empty(),
            "{role:?} holds nothing: the fixture is thin"
        );
        assert_eq!(listed(Filter::InMailbox(role)), want, "{role:?}");
        assert_eq!(
            store.count(&Filter::InMailbox(role), at(0)).unwrap(),
            want.len() as u64,
            "{role:?}"
        );
    }
    for label in [work.id, home.id] {
        let want = by_json(
            &store,
            "EXISTS (SELECT 1 FROM json_each(ts.labels) WHERE value = ?1)",
            &label.to_string(),
        );
        assert!(!want.is_empty());
        assert_eq!(listed(Filter::HasLabel(label)), want);
        assert_eq!(
            store.count(&Filter::HasLabel(label), at(0)).unwrap(),
            want.len() as u64
        );
    }
}

/// The plan SQLite gives `sql`, one line per step.
fn plan(store: &SqliteStore, sql: &str) -> String {
    let db = store.raw_connection();
    db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_search_of_a_mailbox_or_a_label_uses_the_membership_indexes() {
    let store = sqlite();
    delivered(&store, 1, MailboxRole::Inbox);
    // The shapes `sql::compile_knowing` writes for a member that few threads are in, and for one
    // that many are.
    let few_mailbox = plan(
        &store,
        "SELECT count(*) FROM thread_summary ts WHERE ts.thread IN \
         (SELECT tm.thread FROM thread_mailbox tm WHERE tm.mailbox = 'drafts')",
    );
    assert!(few_mailbox.contains("thread_mailbox"), "{few_mailbox}");
    assert!(!few_mailbox.contains("SCAN ts"), "{few_mailbox}");
    let many_mailbox = plan(
        &store,
        "SELECT count(*) FROM thread_summary ts WHERE EXISTS \
         (SELECT 1 FROM thread_mailbox tm WHERE tm.thread = ts.thread AND tm.mailbox = 'inbox')",
    );
    assert!(
        many_mailbox.contains("USING COVERING INDEX") || many_mailbox.contains("PRIMARY KEY"),
        "{many_mailbox}"
    );
    let few_label = plan(
        &store,
        "SELECT count(*) FROM thread_summary ts WHERE ts.thread IN \
         (SELECT tl.thread FROM thread_label tl WHERE tl.label = 'x')",
    );
    assert!(few_label.contains("thread_label_label"), "{few_label}");
    assert!(!few_label.contains("SCAN ts"), "{few_label}");
}
