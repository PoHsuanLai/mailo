//! A search of one account's server, signed in as a sync signs in: an IMAP engine with a fresh
//! credential kept fresh, a Graph engine with a Graph token valid now, or a JMAP engine.

use super::{
    AccountSecrets, ClientRegistry, configured, graph_engine, sending_token, signed_in_imap,
};
use crate::error::CoreError;
use mail_domain::{Filter, Incoming, LabelId};
use mail_runtime::Searched;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use tokio::sync::watch;

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
    async {
        let failed =
            |e: mail_runtime::RuntimeError| CoreError::context("the server was not searched", e);
        match &account.plan.incoming {
            Incoming::Imap { .. } => {
                let (_tx, mut cancel) = watch::channel(false);
                let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
                engine
                    .search_imap(filter, labels, &mut cancel, now)
                    .await
                    .map_err(failed)
            }
            Incoming::Graph => {
                sending_token(&account, secrets.as_ref(), registry, now).await?;
                let mut engine = graph_engine(store, &account, secrets)?;
                engine.search_graph(filter, now).await.map_err(failed)
            }
            Incoming::Jmap { .. } => {
                let mut engine = mail_runtime::JmapEngine::new(
                    account.id,
                    account.plan.clone(),
                    store.clone(),
                    secrets,
                )?;
                engine
                    .search_jmap(filter, labels, now)
                    .await
                    .map_err(failed)
            }
            Incoming::Pop3 { .. } => Err(CoreError::Pop3NoSearch {
                address: account.address.clone(),
            }),
            Incoming::Local => Err(CoreError::LocalNoSearch {
                address: account.address.clone(),
            }),
        }
    }
    .await
}
