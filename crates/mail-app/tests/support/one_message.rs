//! A store with one account, its identity, and one message in the inbox: the composer's tests
//! write beside it and read it back.

#![allow(dead_code)]

use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;

/// The one message's subject.
pub const SUBJECT: &str = "Flight to the conference";

/// The one message's body.
pub const BODY: &str = "The gate changed to B12.";

pub fn account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c7"))
}

/// A store in `dir` with [`account`], its identity, and one message from ada@example.test, an
/// hour old.
pub fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    mail_store::testing::seed_identity_for(
        &store,
        IdentityId::generate(),
        account(),
        "me@example.test",
        None,
    );
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: {SUBJECT}\r\n\
         Date: {date}\r\nMessage-ID: <one@example.test>\r\n\r\n{BODY}\r\n"
    );
    absorb(
        &store,
        account(),
        MailboxRef {
            account: account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "one".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}
