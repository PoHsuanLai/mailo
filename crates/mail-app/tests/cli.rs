//! The CLI against a real store, which is `plan.md` phase 4's "a tiny CLI can list and open".
//!
//! Driven through `cli::run` rather than by spawning the binary, so the assertions are about
//! what the user sees rather than about process plumbing.

use chrono::{TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

// Both modules are pulled in by path: cli.rs calls into account.rs, so the test crate needs
// the same shape the binary has.
#[path = "../src/account.rs"]
mod account;
#[path = "../src/cli.rs"]
mod cli;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn seeded() -> (SqliteStore, tempfile::TempDir, ThreadId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    let mut first = None;
    for (i, subject) in ["lunch on friday", "invoice 2024", "server outage"]
        .iter()
        .enumerate()
    {
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + i as u128));
        if first.is_none() {
            first = Some(thread);
        }
        let raw = store
            .blobs()
            .put(&store.connection(), format!("raw {i}").as_bytes())
            .unwrap();
        let message = Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + i as u128)),
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: Utc
                .timestamp_opt(1_700_000_000 + i as i64 * 3600, 0)
                .unwrap(),
            from: Address {
                name: Some(format!("Sender {i}")),
                email: format!("s{i}@example.test"),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: (*subject).to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{i}@example.test")),
            read: if i == 0 {
                ReadState::Unread
            } else {
                ReadState::Read
            },
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            // The second message has headers only, which is the normal mid-sync state and the
            // one a UI most easily gets wrong by showing an empty body as if it were empty.
            body: if i == 1 {
                Body::Absent
            } else {
                Body::Present {
                    text: Some(format!("the body of {subject}")),
                    raw,
                }
            },
            attachments: vec![],
        };
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .unwrap();
    }
    (store, dir, first.unwrap())
}

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}

#[test]
fn list_shows_threads_newest_first_and_marks_unread() {
    let (store, _dir, _) = seeded();
    let out = cli::run(
        &store,
        &cli::Command::List {
            mailbox: MailboxRole::Inbox,
            limit: 20,
        },
        now(),
    )
    .unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("server outage"), "newest first: {out}");
    assert!(
        lines[2].starts_with('*'),
        "the unread one should be marked: {out}"
    );
}

#[test]
fn an_empty_mailbox_says_so_instead_of_printing_nothing() {
    let (store, _dir, _) = seeded();
    let out = cli::run(
        &store,
        &cli::Command::List {
            mailbox: MailboxRole::Trash,
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(out.contains("no threads in trash"), "{out}");
}

#[test]
fn show_prints_the_body_and_says_when_it_is_not_fetched() {
    let (store, _dir, first) = seeded();
    let out = cli::run(&store, &cli::Command::Show { thread: first }, now()).unwrap();
    assert!(out.contains("lunch on friday"));
    assert!(out.contains("the body of lunch on friday"), "{out}");

    // The headers-only thread must say so rather than render as an empty message, which is how
    // a mid-sync state gets mistaken for a blank email.
    let headers_only = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let out = cli::run(
        &store,
        &cli::Command::Show {
            thread: headers_only,
        },
        now(),
    )
    .unwrap();
    assert!(out.contains("not fetched yet"), "{out}");
}

#[test]
fn search_goes_through_full_text_and_reports_a_miss() {
    let (store, _dir, _) = seeded();
    let hit = cli::run(
        &store,
        &cli::Command::Search {
            needle: "outage".to_owned(),
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(hit.contains("server outage"), "{hit}");

    let miss = cli::run(
        &store,
        &cli::Command::Search {
            needle: "nothingmatchesthis".to_owned(),
            limit: 20,
        },
        now(),
    )
    .unwrap();
    assert!(miss.contains("nothing matches"), "{miss}");
}

#[test]
fn status_counts_unread_separately_from_total() {
    let (store, _dir, _) = seeded();
    let out = cli::run(&store, &cli::Command::Status, now()).unwrap();
    assert!(out.contains("inbox"), "{out}");
    assert!(out.contains("3 total"), "{out}");
    assert!(out.contains("1 unread"), "{out}");
    // Mailboxes with nothing in them are omitted rather than printed as zeroes.
    assert!(!out.contains("spam"), "{out}");
}

#[test]
fn a_missing_thread_is_an_error_not_a_panic() {
    let (store, _dir, _) = seeded();
    let err = cli::run(
        &store,
        &cli::Command::Show {
            thread: ThreadId::generate(),
        },
        now(),
    )
    .expect_err("no such thread");
    assert!(!err.is_empty());
}
