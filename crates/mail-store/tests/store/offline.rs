//! What of an account is held here, and which attachments still wait on the server: counted by
//! both stores, the same way.
//!
//! The parity proptest cannot see this: no filter asks about an attachment's bytes. So every
//! scenario runs against SQLite and memory, compares, and names the numbers it must produce.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{MemoryStore, Offline, RemotePart, SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
fn acct_other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}

const MB: u64 = 1024 * 1024;

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

/// Raw blobs every scenario may name, stored in the SQLite store so its foreign key holds; the
/// in-memory store never looks one up.
struct Blobs {
    raw: Vec<BlobId>,
}

/// Run `scenario` on both stores and hand back what each produced.
fn both<T>(scenario: impl Fn(&dyn Store, &Blobs) -> T) -> (T, T) {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    for (account, address) in [
        (acct_account(), "me@example.test"),
        (acct_other(), "also@example.test"),
    ] {
        sqlite
            .raw_connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
    }
    let blobs = Blobs {
        raw: (0..8)
            .map(|n| sqlite.blobs().put(format!("raw {n}").as_bytes()).unwrap())
            .collect(),
    };
    let memory = MemoryStore::new();
    (scenario(&sqlite, &blobs), scenario(&memory, &blobs))
}

fn imap(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 7,
        uid,
    }
}

fn remote(name: &str, section: &str, size: u64) -> Attachment {
    Attachment {
        name: name.to_owned(),
        mime: "application/pdf".to_owned(),
        size,
        content: PartContent::Remote {
            section: section.to_owned(),
        },
        inline: Inline::Attached,
    }
}

fn held(name: &str, blob: BlobId, size: u64) -> Attachment {
    Attachment {
        name: name.to_owned(),
        mime: "image/png".to_owned(),
        size,
        content: PartContent::Held(blob),
        inline: Inline::Attached,
    }
}

fn message(
    account: AccountId,
    n: u128,
    body: Option<BlobId>,
    attachments: Vec<Attachment>,
) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000)),
        account,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match body {
            Some(raw) => Body::Present {
                text: Some(format!("body {n}")),
                raw,
            },
            None => Body::Absent,
        },
        attachments,
    }
}

fn deliver(store: &dyn Store, m: &Message, at: RemoteRef, raw: BlobId) {
    let RemoteRef::Imap { mailbox: path, .. } = &at else {
        unreachable!("IMAP only here")
    };
    store
        .ingest(
            m.account.clone(),
            Ingest {
                mailbox: MailboxRef {
                    account: m.account.clone(),
                    path: path.clone(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: at.clone(),
                    key: m.key.clone(),
                    raw,
                    message: m.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: "INBOX".to_owned(),
    }
}

fn id(n: u128) -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(n))
}

/// One account with each kind of message, and another account's mail beside it:
///
/// - 1, headers only, in the inbox;
/// - 2, whole, with an image it holds, in the inbox;
/// - 3, rebuilt, two parts on the server (5 MB as "2", 2 MB as "3"), in the inbox;
/// - 4, rebuilt, one part on the server (3 MB), in Archive only;
/// - 5, rebuilt, newer than 3, one part on the server as large as 3's smaller one, in the inbox;
/// - 6, the other account's, rebuilt, in its inbox.
fn seed(store: &dyn Store, blobs: &Blobs) {
    let raw = &blobs.raw;
    deliver(
        store,
        &message(acct_account(), 1, None, vec![]),
        imap("INBOX", 1),
        raw[0],
    );
    deliver(
        store,
        &message(
            acct_account(),
            2,
            Some(raw[1]),
            vec![held("a.png", raw[6], 900)],
        ),
        imap("INBOX", 2),
        raw[1],
    );
    deliver(
        store,
        &message(
            acct_account(),
            3,
            Some(raw[2]),
            vec![
                remote("big.pdf", "2", 5 * MB),
                remote("mid.pdf", "3", 2 * MB),
            ],
        ),
        imap("INBOX", 3),
        raw[2],
    );
    deliver(
        store,
        &message(
            acct_account(),
            4,
            Some(raw[3]),
            vec![remote("old.zip", "2", 3 * MB)],
        ),
        imap("Archive", 4),
        raw[3],
    );
    deliver(
        store,
        &message(
            acct_account(),
            5,
            Some(raw[4]),
            vec![remote("new.pdf", "2", 2 * MB)],
        ),
        imap("INBOX", 5),
        raw[4],
    );
    deliver(
        store,
        &message(
            acct_other(),
            6,
            Some(raw[5]),
            vec![remote("theirs.pdf", "2", MB)],
        ),
        imap("INBOX", 6),
        raw[5],
    );
}

fn part(n: u128, section: &str, size: u64) -> RemotePart {
    RemotePart {
        message: id(n),
        section: section.to_owned(),
        size,
    }
}

#[test]
fn the_counts_say_what_is_held_and_what_waits() {
    let (sql, mem) = both(|store, blobs| {
        seed(store, blobs);
        (
            store.offline(acct_account()).unwrap(),
            store.offline(acct_other()).unwrap(),
        )
    });
    assert_eq!(sql, mem);
    assert_eq!(
        sql.0,
        Offline {
            messages: 5,
            // Only 2: 1 has no body, and 3, 4 and 5 each wait on a part.
            held: 1,
            parts_remote: 4,
            remote_bytes: 12 * MB,
        }
    );
    assert_eq!(
        sql.1,
        Offline {
            messages: 1,
            held: 0,
            parts_remote: 1,
            remote_bytes: MB,
        }
    );
}

#[test]
fn a_mailboxs_parts_come_smallest_first_and_newest_first_within_a_size() {
    let (sql, mem) = both(|store, blobs| {
        seed(store, blobs);
        (
            store.remote_parts_in(&inbox(), 10).unwrap(),
            store.remote_parts_in(&inbox(), 1).unwrap(),
        )
    });
    assert_eq!(sql, mem);
    // Archive's part is not the inbox's, and the other account's is not this one's.
    assert_eq!(
        sql.0,
        vec![
            part(5, "2", 2 * MB),
            part(3, "3", 2 * MB),
            part(3, "2", 5 * MB)
        ]
    );
    assert_eq!(
        sql.1,
        vec![part(5, "2", 2 * MB)],
        "the limit cuts the largest"
    );
}

#[test]
fn holding_every_part_takes_it_off_the_list_and_counts_the_message_held() {
    let (sql, mem) = both(|store, blobs| {
        seed(store, blobs);
        let before = store.offline(acct_account()).unwrap();
        for p in store.remote_parts_in(&inbox(), 10).unwrap() {
            store
                .hold_part(p.message, &p.section, blobs.raw[7], 100)
                .unwrap();
        }
        (
            before,
            store.offline(acct_account()).unwrap(),
            store.remote_parts_in(&inbox(), 10).unwrap(),
        )
    });
    assert_eq!(sql, mem);
    let (before, after, left) = sql;
    assert_eq!(after.held, before.held + 2, "3 and 5 are now whole here");
    assert_eq!(after.parts_remote, before.parts_remote - 3);
    assert_eq!(after.remote_bytes, 3 * MB, "Archive's part still waits");
    assert_eq!(left, Vec::<RemotePart>::new());
}
