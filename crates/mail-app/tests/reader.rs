//! What the reader shows for a real message, including the ones that are broken.
//!
//! The sandboxed iframe in `ui.rs` had never received anything: the reader passed `None` for the
//! HTML part on every message, because the part is not a column — it lives inside the stored raw
//! bytes. So every HTML message in the world rendered as its plain-text alternative, and one
//! with no text alternative rendered as nothing at all.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use mail_store::{SqliteStore, Store};

#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

#[path = "../src/reader.rs"]
mod reader;

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

/// Ingest `raw`, with `text` as the extracted plain part.
fn ingest(store: &SqliteStore, raw_bytes: &[u8], text: Option<&str>) -> Message {
    let raw = store.blobs().put(&store.connection(), raw_bytes).unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("{}@example.test", uuid::Uuid::new_v4())),
        date: at(0),
        from: Address {
            name: None,
            email: "sender@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "subject".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: text.map(str::to_owned),
            raw,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: SyncCursor::Pop,
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: uuid::Uuid::new_v4().to_string(),
                    },
                    key: message.key.clone(),
                    raw,
                    message: message.clone(),
                }],
                flags: vec![],
                labels: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    message
}

const MULTIPART: &[u8] = b"From: sender@example.test\r\n\
Subject: subject\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
the plain alternative\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>the <b>rich</b> alternative</p>\r\n\
--b1--\r\n";

#[test]
fn an_html_message_reaches_the_sandbox_instead_of_falling_back_to_text() {
    let (store, _dir) = store();
    let message = ingest(&store, MULTIPART, Some("the plain alternative"));

    let html = reader::html_of(&store, &message).expect("the message has an HTML part");
    assert!(html.contains("rich"), "{html}");

    match view::reading(&message.body, Some(&html), policy()) {
        view::Reading::Html(rendered) => {
            assert!(rendered.contains("rich"), "{rendered}");
            assert!(
                rendered.contains("<b>"),
                "formatting was stripped: {rendered}"
            );
        }
        other => panic!("an HTML message rendered as {other:?}"),
    }
}

#[test]
fn a_plain_text_message_has_no_html_part_and_renders_as_text() {
    let (store, _dir) = store();
    let raw = b"From: sender@example.test\r\nSubject: s\r\n\r\njust words\r\n";
    let message = ingest(&store, raw, Some("just words"));

    assert_eq!(reader::html_of(&store, &message), None);
    match view::reading(&message.body, None, policy()) {
        view::Reading::Text(text) => assert_eq!(text, "just words"),
        other => panic!("plain text rendered as {other:?}"),
    }
}

#[test]
fn a_message_whose_bytes_do_not_parse_is_still_readable() {
    // A blank pane for mail every other client displays is worse than a rough rendering.
    let (store, _dir) = store();
    let message = ingest(
        &store,
        b"\x00\x01\x02 not a message at all",
        Some("recovered text"),
    );

    assert_eq!(
        reader::html_of(&store, &message),
        None,
        "unparseable bytes must not raise"
    );
    match view::reading(&message.body, None, policy()) {
        view::Reading::Text(text) => assert_eq!(text, "recovered text"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_message_with_no_body_yet_is_not_mistaken_for_an_empty_one() {
    // Mid-sync. `Body::Absent` has no blob to read, and the reader must say "not downloaded"
    // rather than showing an empty message.
    let (store, _dir) = store();
    let mut message = ingest(&store, MULTIPART, None);
    message.body = Body::Absent;

    assert_eq!(reader::html_of(&store, &message), None);
    assert_eq!(
        view::reading(&message.body, None, policy()),
        view::Reading::NotFetched
    );
}

#[test]
fn a_script_in_the_html_part_does_not_survive_to_the_iframe() {
    // The sanitizer has its own adversarial tests; this one proves the reader actually routes
    // through it rather than handing raw bytes to the frame.
    let (store, _dir) = store();
    let raw = b"From: sender@example.test\r\n\
Subject: s\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>hello</p><script>alert(1)</script>\r\n";
    let message = ingest(&store, raw, None);

    let html = reader::html_of(&store, &message).expect("an HTML part");
    assert!(
        html.contains("<script>"),
        "the raw part should be untouched"
    );

    match view::reading(&message.body, Some(&html), policy()) {
        view::Reading::Html(rendered) => {
            assert!(rendered.contains("hello"), "{rendered}");
            assert!(
                !rendered.contains("<script"),
                "a script reached the frame: {rendered}"
            );
        }
        other => panic!("{other:?}"),
    }
}
