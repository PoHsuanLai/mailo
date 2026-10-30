//! The account the command line's tests share with the window's (`mail-app/tests/support/store.rs`,
//! which seeds whole stores): one id, so a message id or a draft means the same in both.

use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;

/// The account every test's store holds.
pub fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
