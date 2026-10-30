//! The account, identity and clock most tests here start from.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{IdentityId, MailboxRef};
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

/// The identity that account sends as.
pub const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

/// A fixed moment, so nothing a test stores depends on when it ran.
pub fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}
