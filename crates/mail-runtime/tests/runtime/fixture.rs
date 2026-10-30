//! The account, identity, clock and store most tests here start from.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::MailboxRef;
use mail_domain::id::account_id_from_uuid;
use mail_store::SqliteStore;
use porter_core::AccountId;

/// The one account a test syncs, unless it says otherwise.
pub fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

/// That account's inbox.
pub fn inbox() -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: "INBOX".to_owned(),
    }
}

/// A fixed moment, so nothing a test stores depends on when it ran.
pub fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// A store in a directory of its own, holding [`acct_account`] as `me@example.test`. The
/// directory lives as long as the second half does.
pub fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    (store, dir)
}
