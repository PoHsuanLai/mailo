//! Mail stored before raw 8-bit headers were decoded is re-read from its bytes, once.
//!
//! The database is built at the current version, then 0012 is un-applied and the store opened
//! again, so the migration runs over rows exactly as an upgrade would find them.

use chrono::{TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

/// A message whose subject and sender's name are raw GBK, as an old server stores them.
const GBK_RAW: &[u8] = b"From: \xcd\xf5\xd0\xa1\xc3\xf7 <wang@example.test>\r\n\
To: me@example.test\r\n\
Subject: \xb9\xd8\xd3\xda\xcf\xc2\xd6\xdc\xcf\xee\xc4\xbf\xbb\xe1\xd2\xe9\xb5\xc4\xb0\xb2\xc5\xc5\r\n\
Message-ID: <gbk@example.test>\r\n\
Content-Type: text/plain; charset=gbk\r\n\
\r\n\
body\r\n";

fn message(n: u128, subject: &str, body: Body) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: Utc.timestamp_opt(1_700_000_000 + n as i64, 0).unwrap(),
        from: Address {
            // What the old parser made of the GBK name.
            name: Some("\u{FFFD}\u{FFFD}\u{FFFD}".to_owned()),
            email: "wang@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Read,
        star: Star::Starred,
        mailbox: MailboxRole::Archive,
        labels: vec![],
        body,
        attachments: vec![],
    }
}

#[test]
fn garbled_messages_held_whole_are_re_read_and_the_rest_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("mail.db");
    let blobs = dir.path().join("blobs");
    let (garbled, clean, headers_only) = {
        let store = SqliteStore::open(&db_path, &blobs).unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();
        let raw = store.blobs().put(&store.connection(), GBK_RAW).unwrap();
        let held = || Body::Present {
            text: Some("body".to_owned()),
            raw,
        };
        let garbled = message(1, "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}", held());
        let clean = Message {
            from: Address {
                name: Some("Ada".to_owned()),
                email: "ada@example.test".to_owned(),
            },
            ..message(2, "lunch", held())
        };
        let headers_only = message(3, "\u{FFFD}\u{FFFD}", Body::Absent);
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: [&garbled, &clean, &headers_only]
                        .into_iter()
                        .map(|m| Change::MessageUpsert(Box::new(m.clone())))
                        .collect(),
                },
            )
            .unwrap();
        (garbled.id, clean.id, headers_only.id)
    };

    // Un-apply 0012, as a database last opened by the previous build would be — and every
    // migration after it, since the version is the highest one applied and a later one would
    // leave 0012 looking done.
    {
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "DROP TABLE messages_to_reparse;
             DROP TABLE contacts; DROP TABLE contacts_counted; DROP TABLE contacts_sent;
             DROP TABLE address_books; DROP TABLE contacts_to_backfill;
             DROP TABLE templates;
             DROP TABLE invite_answers;
             DROP TABLE rules; DROP TABLE vacations;
             DROP TABLE pgp_keys; DROP TABLE autocrypt_peers;
             ALTER TABLE drafts DROP COLUMN openpgp;
             DELETE FROM schema_version WHERE version >= 12;",
        )
        .unwrap();
    }

    let store = SqliteStore::open(&db_path, &blobs).unwrap();
    assert_eq!(
        store.reparse_queue().unwrap(),
        vec![garbled],
        "only the garbled message whose bytes are here"
    );

    assert_eq!(mail_runtime::reparse_queued(&store).unwrap(), 1);
    let fixed = store.message(garbled).unwrap();
    assert_eq!(fixed.subject, "关于下周项目会议的安排");
    assert_eq!(fixed.from.name.as_deref(), Some("王小明"));
    assert_eq!(fixed.to[0].email, "me@example.test");
    // What the user did to it is not undone by its being re-read.
    assert_eq!(fixed.star, Star::Starred);
    assert_eq!(fixed.read, ReadState::Read);
    assert_eq!(fixed.mailbox, MailboxRole::Archive);
    // The list reads the summary, not the message.
    assert_eq!(
        store.thread(fixed.thread).unwrap().summary.subject,
        "关于下周项目会议的安排"
    );

    assert_eq!(store.message(clean).unwrap().subject, "lunch");
    assert_eq!(
        store.message(headers_only).unwrap().subject,
        "\u{FFFD}\u{FFFD}",
        "rewritten when its body arrives, not before"
    );
    assert!(store.reparse_queue().unwrap().is_empty(), "done once");
    assert_eq!(mail_runtime::reparse_queued(&store).unwrap(), 0);
}
