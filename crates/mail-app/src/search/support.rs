//! Fixtures for the search tests. Not compiled into the binary.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};

pub const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

pub fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0)
        .single()
        .expect("fixture timestamp")
}

pub fn summary(
    n: u128,
    subject: &str,
    snippet: &str,
    email: &str,
    secs: i64,
    read: ReadState,
    star: Star,
) -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::from_uuid(uuid::Uuid::from_u128(n)),
        account: ACCOUNT,
        subject: subject.to_owned(),
        snippet: snippet.to_owned(),
        from: Address {
            name: None,
            email: email.to_owned(),
        },
        participants: vec![],
        recipients: vec![],
        last_date: at(secs),
        message_count: 1,
        read,
        star,
        mailboxes: MailboxSet::only(MailboxRole::Inbox),
        labels: vec![],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
    }
}

pub fn remember(store: &MemoryStore, n: u128, subject: &str, body: &str, secs: i64) {
    let message = message(n, subject, body, secs, ReadState::Read, Star::Unstarred);
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message))],
            },
        )
        .expect("memory apply");
}

pub fn message(
    n: u128,
    subject: &str,
    body: &str,
    secs: i64,
    read: ReadState,
    star: Star,
) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@b.c")),
        date: at(secs),
        from: Address {
            name: None,
            email: "a@b.c".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@b.c")),
        read,
        star,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(body.to_owned()),
            raw: BlobId::from_uuid(uuid::Uuid::from_u128(0xB000 + n)),
        },
        attachments: vec![],
    }
}

/// A sqlite store with one account, so `mailo search` and [`super::run`] see the same rows.
pub fn sqlite_with(rows: &[(&str, &str, i64)]) -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SqliteStore::in_memory(dir.path()).expect("sqlite");
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .expect("account");
    for (i, (subject, body, secs)) in rows.iter().enumerate() {
        let raw = store
            .blobs()
            .put(&store.connection(), body.as_bytes())
            .expect("blob");
        let mut message = message(
            i as u128,
            subject,
            body,
            *secs,
            ReadState::Read,
            Star::Unstarred,
        );
        if let Body::Present { raw: slot, .. } = &mut message.body {
            *slot = raw;
        }
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .expect("sqlite apply");
    }
    (store, dir)
}
