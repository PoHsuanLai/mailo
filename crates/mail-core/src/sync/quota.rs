//! How much of one account's storage its server says is used, signed in as a sync signs in by
//! the account's protocol ([`super::access`]).
//!
//! IMAP reads it from `QUOTA` (RFC 9208), JMAP from `Quota/get` (RFC 9425). Microsoft Graph
//! keeps a mailbox's size behind the reports API, which needs an administrator's consent, and
//! POP3 and local folders have nothing to say: for those, and for a server that keeps no quota,
//! there is none.

use super::access::access;
use super::run::Run;
use super::{configured, pass_error};
use crate::error::CoreError;
use mail_proto::Storage;
use porter_core::AccountId;

impl crate::mail::SyncOps<'_> {
    /// `account`'s storage as its server counts it: `None` where the server keeps no quota or
    /// there is no server to ask.
    pub async fn quota(&self, account: AccountId) -> Result<Option<Storage>, CoreError> {
        let mail = self.0;
        let (store, secrets, now) = (mail.store(), mail.secrets(), mail.now());
        let registry = mail.clients()?;
        let account = configured(store)?
            .into_iter()
            .find(|a| a.id == account)
            .ok_or(CoreError::AccountGone)?;
        let run = Run {
            store,
            secrets,
            registry: &registry,
            now,
            account: &account,
        };
        access(&account.plan.incoming)
            .quota(&run)
            .await
            .map_err(pass_error)
    }
}
