//! What the reader costs on a conversation that is actually long.
//!
//! Every other test of the reader opens a thread with one or two messages in it, which says
//! nothing about a mailing-list thread with two hundred. The shell re-runs `reader::render` for
//! every message in the open thread on *every* render — a keystroke in the search box, a hover
//! action, a sync landing — so whatever one message costs is paid two hundred times, several
//! times a second.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::reader;
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use mail_store::{SqliteStore, Store};
use std::time::Instant;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn policy() -> SanitizePolicy {
    SanitizePolicy {
        remote_images: RemoteImages::Blocked,
        version: SanitizePolicy::CURRENT.version,
    }
}

/// An HTML message with an inline image, which is the expensive shape.
fn raw_message(n: i64, padding: usize) -> Vec<u8> {
    let filler = "x".repeat(padding);
    format!(
        "From: sender{n}@example.test\r\n\
         Subject: re: the long thread\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/related; boundary=\"b\"\r\n\
         \r\n\
         --b\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         \r\n\
         <p>message {n}</p><img src=\"cid:pic@example\"><p>{filler}</p>\r\n\
         --b\r\n\
         Content-Type: image/png\r\n\
         Content-ID: <pic@example>\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         iVBORw0KGgo=\r\n\
         --b--\r\n"
    )
    .into_bytes()
}

/// One thread of `count` messages.
fn thread_of(store: &SqliteStore, count: i64, padding: usize) -> Vec<Message> {
    let thread = ThreadId::generate();
    let mut built = Vec::new();
    let mut fetched = Vec::new();
    for n in 0..count {
        let raw = store
            .blobs()
            .put(&store.connection(), &raw_message(n, padding))
            .unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: at(n),
            from: Address {
                name: None,
                email: format!("sender{n}@example.test"),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "re: the long thread".to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some(format!("message {n}")),
                raw,
            },
            attachments: vec![],
        };
        fetched.push(Fetched {
            remote: RemoteRef::Pop {
                uidl: format!("u{n}"),
            },
            key: message.key.clone(),
            raw,
            message: message.clone(),
        });
        built.push(message);
    }
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: fetched,
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    built
}

fn store() -> (SqliteStore, tempfile::TempDir) {
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
    (store, dir)
}

fn render_all(store: &SqliteStore, messages: &[Message]) -> std::time::Duration {
    let start = Instant::now();
    for message in messages {
        let reading = reader::render(store, message, policy());
        std::hint::black_box(&reading);
    }
    start.elapsed()
}

#[test]
fn opening_a_long_thread_is_not_paid_twice_per_message() {
    // The shell re-renders the open thread on every revision, so this is the budget for a
    // keystroke — not for opening a conversation once.
    let (store, _dir) = store();
    let messages = thread_of(&store, 200, 2_000);

    let elapsed = render_all(&store, &messages);
    let each = elapsed / messages.len() as u32;
    eprintln!("200-message thread: {elapsed:?} total, {each:?} each");

    assert!(
        elapsed < std::time::Duration::from_millis(400),
        "rendering a 200-message thread took {elapsed:?}; the shell does this on every render"
    );
}

#[test]
fn a_message_with_a_large_body_is_not_parsed_more_than_it_must_be() {
    // A 2MB message is ordinary once someone sends a deck. Parsing it twice per render is the
    // difference between a reader that opens and one that stalls.
    let (store, _dir) = store();
    let messages = thread_of(&store, 5, 2_000_000);

    let elapsed = render_all(&store, &messages);
    eprintln!("five 2MB messages: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "rendering five large messages took {elapsed:?}"
    );
}
