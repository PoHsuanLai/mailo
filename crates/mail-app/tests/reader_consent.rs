//! Consent to remote images is not something the reader's cache can carry from one render to the
//! next: it is the window's `Shell` that revokes it, so this reads the cache through one.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::ui::view;
use mail_core::reader;
use mail_domain::*;
use mail_mime::{Block, ImgSrc};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn remote_count(reading: &reader::Reading) -> usize {
    reading
        .document()
        .map(|document| count_remote(&document.blocks))
        .unwrap_or(0)
}

fn count_remote(blocks: &[Block]) -> usize {
    let mut count = 0;
    let mut stack: Vec<&[Block]> = vec![blocks];
    while let Some(level) = stack.pop() {
        for block in level {
            match block {
                Block::Image {
                    src: ImgSrc::Remote(_),
                    ..
                } => count += 1,
                Block::Quote { blocks, .. } | Block::Signature(blocks) => stack.push(blocks),
                Block::List { items, .. } => {
                    for item in items {
                        stack.push(item);
                    }
                }
                _ => {}
            }
        }
    }
    count
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
                cursor: Some(SyncCursor::Pop),
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
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    message
}

const REMOTE: &[u8] = b"From: ada@example.test\r\n\
    Subject: pictures\r\n\
    MIME-Version: 1.0\r\n\
    Content-Type: text/html; charset=utf-8\r\n\r\n\
    <p>hello</p><img src=\"https://tracker.test/pixel.gif\">\r\n";

#[test]
fn consent_is_not_cached_across_policies() {
    // The count is the difference the action makes: blocked, then allowed, then
    // revoked by opening another thread. A cache that forgot the policy would
    // still be holding the allowed image on the third render.
    reader::forget_everything();
    let (store, _dir) = store();
    let message = ingest(&store, REMOTE, Some("hello"));
    let mut shell = view::Shell::default();

    let blocked = reader::render(&store, &message, shell.policy());
    assert_eq!(remote_count(&blocked), 0, "blocked: {blocked:?}");

    shell.show_remote_images = true;
    let allowed = reader::render(&store, &message, shell.policy());
    assert_eq!(remote_count(&allowed), 1, "allowed: {allowed:?}");

    shell.open(ThreadId::generate());
    let revoked = reader::render(&store, &message, shell.policy());
    assert_eq!(
        remote_count(&revoked),
        0,
        "opening another thread served the allowed image: {revoked:?}"
    );
}
