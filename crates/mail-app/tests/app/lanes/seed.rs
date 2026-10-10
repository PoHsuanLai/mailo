//! The store every lane starts from: one POP3 account with its identity and capabilities, and
//! an inbox of four conversations from named senders, so the contact book knows them.

use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::path::Path;
use std::sync::Arc;

/// The one account every lane's store holds.
pub fn account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

/// The account's own address.
pub const ME: &str = "me@example.test";

/// The seeded inbox, newest first: (sender, subject). Every sender is named, so the contact
/// book learns them as mail teaches it.
pub const INBOX: [(&str, &str); 4] = [
    (
        "Ada Lovelace <ada@example.test>",
        "Flight to the conference",
    ),
    (
        "Grace Hopper <grace@example.test>",
        "The invoice for September",
    ),
    ("Alan Turing <alan@example.test>", "Lunch on Thursday"),
    (
        "Edsger Dijkstra <edsger@example.test>",
        "Notes from the review",
    ),
];

/// Deliver `raw` into the Inbox as POP3 does, `uidl` naming it.
pub fn deliver(store: &SqliteStore, uidl: &str, raw: &str) {
    absorb(
        store,
        account(),
        MailboxRef {
            account: account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: raw.as_bytes().to_vec(),
        }],
        false,
        chrono::Utc::now(),
    )
    .expect("the message is absorbed");
}

/// A date header `hours` before now.
pub fn hours_ago(hours: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc2822()
}

/// A store in `dir` with one account, its identity and capabilities, and [`INBOX`].
pub fn seeded(dir: &Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).expect("a blob directory");
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).expect("the store opens");
    {
        mail_store::testing::seed_account(&store, account(), ME);
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            account(),
            ME,
            Some("Me"),
        );
        let caps = AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: FolderRoles::default(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Yes,
            connections: ConnectionBudget::default(),
            observed_at: chrono::Utc::now(),
        };
        mail_store::testing::seed_caps(&store, account(), &caps, chrono::Utc::now()).unwrap();
    }
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let raw = format!(
            "From: {from}\r\nTo: Me <{ME}>\r\nSubject: {subject}\r\nDate: {}\r\n\
             Message-ID: <seed{n}@example.test>\r\n\r\nThe body of {subject}.\r\n",
            hours_ago(n as i64 + 1)
        );
        deliver(&store, &format!("seed{n}"), &raw);
    }
    Arc::new(store)
}

/// The copy of a sent message in the Sent folder, as the next sync after the send brings it.
pub fn sent_copy(store: &SqliteStore, uidl: &str, raw: &[u8]) {
    mail_runtime::absorb_into(
        store,
        account(),
        mail_runtime::Destination {
            mailbox: MailboxRef {
                account: account(),
                path: "Sent".to_owned(),
            },
            role: MailboxRole::Sent,
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: raw.to_vec(),
        }],
        false,
        chrono::Utc::now(),
    )
    .expect("the Sent copy is absorbed");
}
