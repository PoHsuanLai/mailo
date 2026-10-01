//! A search of one account's server, signed in as a sync signs in: an IMAP engine with a fresh
//! credential kept fresh, a Graph engine with a Graph token valid now, or a JMAP engine.

use super::{
    KeyringSecrets, OAuthRegistry, Secrets, configured, graph_engine, sending_token, signed_in_imap,
};
use mail_domain::{AccountId, Filter, Incoming, LabelId};
use mail_runtime::Searched;
use mail_store::SqliteStore;
use std::sync::Arc;
use tokio::sync::watch;

/// Search `account`'s server for `filter`, and keep what it finds here.
///
/// `labels` is the account's labels with their names, for `label:`. Blocking, with a runtime of
/// its own, like [`super::fetch_part`]: its caller is a click, not async code.
pub fn search_server(
    store: &Arc<SqliteStore>,
    account: AccountId,
    filter: &Filter,
    labels: &[(LabelId, String)],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Searched, String> {
    let registry = OAuthRegistry::load_default().map_err(|e| e.to_string())?;
    search_server_with(
        store,
        Arc::new(KeyringSecrets),
        &registry,
        account,
        filter,
        labels,
        now,
    )
}

/// The same, with the secret store named, so a test can run it.
pub fn search_server_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    registry: &OAuthRegistry,
    account: AccountId,
    filter: &Filter,
    labels: &[(LabelId, String)],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Searched, String> {
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or_else(|| "that account is no longer configured".to_owned())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let failed = |e: mail_runtime::RuntimeError| format!("the server was not searched: {e}");
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
                )
                .map_err(|e| e.to_string())?;
                engine
                    .search_jmap(filter, labels, now)
                    .await
                    .map_err(failed)
            }
            Incoming::Pop3 { .. } => Err(format!(
                "{} is POP3, which has one mailbox and no search",
                account.address
            )),
            Incoming::Local => Err(format!(
                "{} is kept on this computer: there is no server to search",
                account.address
            )),
        }
    })
}
