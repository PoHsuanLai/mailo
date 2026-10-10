//! The async entry points, waited on from a plain test thread.
//!
//! Most of these tests stand a server up on a thread of their own and read the answer in a
//! `#[test]`; the library is async and starts no runtime, so each call here is awaited on a
//! runtime made for it, as the application's edge does with its own. A test that is `async`
//! itself calls the library directly instead.

use mail_core::CoreError;
use mail_core::sync::{self, live, report::Hooks, report::PassEnd};
use mail_domain::MessageId;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Wait for `work` on a current-thread runtime of its own.
pub fn block_on<T>(work: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the test")
        .block_on(work)
}

pub fn run_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, CoreError> {
    block_on(sync::run_with(store, secrets, registry, now, hooks))
}

pub fn folder_now_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    path: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PassEnd, CoreError> {
    block_on(sync::folder_now_with(
        store, secrets, registry, account, path, now,
    ))
}

pub fn fetch_part_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    message: MessageId,
    section: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    block_on(sync::fetch_part_with(
        store, secrets, registry, message, section, now,
    ))
}

pub fn fetch_body_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    message: MessageId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), (mail_domain::Retry, String)> {
    block_on(sync::fetch_body_with(
        store, secrets, registry, message, now,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn listen_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    cancel: watch::Receiver<bool>,
    hold: Option<live::Hold>,
    grace: Duration,
    heard: &dyn Fn(live::Heard),
) -> Result<(), live::Lost> {
    block_on(live::listen_with(
        store, secrets, registry, account, cancel, hold, grace, heard,
    ))
}
