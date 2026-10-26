//! How much of one account's storage its server says is used, signed in as a sync signs in: an
//! IMAP engine with a fresh credential kept fresh, or a JMAP engine.
//!
//! IMAP reads it from `QUOTA` (RFC 9208), JMAP from `Quota/get` (RFC 9425). Microsoft Graph
//! keeps a mailbox's size behind the reports API, which needs an administrator's consent, and
//! POP3 and local folders have nothing to say: for those, and for a server that keeps no quota,
//! there is none.

use super::{AccountSecrets, ClientRegistry, configured, signed_in_imap};
use crate::error::CoreError;
use mail_domain::Incoming;
use mail_proto::Storage;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use tokio::sync::watch;

impl crate::mail::SyncOps<'_> {
    /// `account`'s storage as its server counts it: `None` where the server keeps no quota or
    /// there is no server to ask.
    pub async fn quota(&self, account: AccountId) -> Result<Option<Storage>, CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        quota_with(mail.store(), mail.secrets(), &registry, account, mail.now()).await
    }
}

/// [`SyncOps::quota`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn quota_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<Storage>, CoreError> {
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or(CoreError::AccountGone)?;
    let failed = |e: mail_runtime::RuntimeError| {
        CoreError::context("the server did not say how much storage is used", e)
    };
    match &account.plan.incoming {
        Incoming::Imap { .. } => {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
            engine.quota_imap(&mut cancel).await.map_err(failed)
        }
        Incoming::Jmap { .. } => {
            let mut engine = mail_runtime::JmapEngine::new(
                account.id,
                account.plan.clone(),
                store.clone(),
                secrets,
            )?;
            engine.quota_jmap().await.map_err(failed)
        }
        Incoming::Graph | Incoming::Pop3 { .. } | Incoming::Local => Ok(None),
    }
}

/// Whether an account's server can say how much storage is used: IMAP and JMAP.
pub fn has_quota(incoming: &Incoming) -> bool {
    matches!(incoming, Incoming::Imap { .. } | Incoming::Jmap { .. })
}
