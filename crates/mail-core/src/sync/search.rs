//! A search of one account's server, signed in as a sync signs in by the account's protocol
//! ([`super::access`]).

use super::access::access;
use super::run::Run;
use super::{AccountSecrets, ClientRegistry, configured, pass_error};
use crate::error::CoreError;
use mail_domain::{Filter, LabelId};
use mail_runtime::Searched;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

impl crate::mail::SyncOps<'_> {
    /// Search `account`'s server for `filter`, and keep what it finds here.
    ///
    /// `labels` is the account's labels with their names, for `label:`.
    pub async fn search_server(
        &self,
        account: AccountId,
        filter: &Filter,
        labels: &[(LabelId, String)],
    ) -> Result<Searched, CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        search_server_with(
            mail.store(),
            mail.secrets(),
            &registry,
            account,
            filter,
            labels,
            mail.now(),
        )
        .await
    }
}

/// [`SyncOps::search_server`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn search_server_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    filter: &Filter,
    labels: &[(LabelId, String)],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Searched, CoreError> {
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or(CoreError::AccountGone)?;
    let run = Run {
        store,
        secrets,
        registry,
        now,
        account: &account,
    };
    access(&account.plan.incoming)
        .search(&run, filter, labels)
        .await
        .map_err(pass_error)
}
