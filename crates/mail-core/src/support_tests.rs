//! What more than one of the crate's unit tests starts from. Not compiled into the binary, and
//! named as a test file so the checks over the program's source pass it by.

use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};

pub use mail_domain::testing::{account, now};

/// A POP3 account's setup at `example.edu`, signing in as `login` (the address when `None`).
pub fn pop3(login: Option<&str>) -> crate::account::Setup {
    crate::account::Setup::Pop3(mail_domain::presets::ManualPop3 {
        pop3_host: "pop.example.edu".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.example.edu".to_owned(),
        smtp_port: 465,
        login: login.map(str::to_owned),
    })
}

/// A plain IMAP server's capabilities: no labels, threads, extensions or pipelining, one
/// connection, polled every five minutes. Read at [`now`], so a pass at [`now`] finds them fresh
/// and does not ask the server again.
pub fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

/// [`account`]'s incoming password, kept in `secrets`.
pub fn with_password(secrets: &porter_secrets::MemorySecrets, password: &str) {
    use mail_secrets::AccountSecrets as _;
    mail_secrets::block_on(secrets.put(
        &porter_core::SecretKey {
            account: account(),
            purpose: porter_core::SecretPurpose::IncomingPassword,
        },
        &porter_core::Credential::Password(porter_core::SecretText::new(password.to_owned())),
    ))
    .unwrap();
}

/// `secs` seconds after [`now`].
pub fn at(secs: i64) -> DateTime<Utc> {
    now() + TimeDelta::seconds(secs)
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
        account: account(),
        subject: subject.to_owned(),
        snippet: snippet.to_owned(),
        from: Address {
            name: None,
            email: email.to_owned(),
        },
        last_date: at(secs),
        read,
        star,
        ..mail_domain::testing::summary()
    }
}

pub fn remember(store: &MemoryStore, n: u128, subject: &str, body: &str, secs: i64) {
    let message = message(n, subject, body, secs, ReadState::Read, Star::Unstarred);
    store
        .apply(
            account(),
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
        account: account(),
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
        flag: mail_domain::Hue::Red,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(body.to_owned()),
            raw: BlobId::from_uuid(uuid::Uuid::from_u128(0xB000 + n)),
        },
        attachments: vec![],
    }
}

/// A sqlite store with one account, so `mailo search` and [`crate::search::run`] see the same
/// rows.
pub fn sqlite_with(rows: &[(&str, &str, i64)]) -> SqliteStore {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    for (i, (subject, body, secs)) in rows.iter().enumerate() {
        let raw = store.blobs().put(body.as_bytes()).expect("blob");
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
                account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .expect("sqlite apply");
    }
    store
}
