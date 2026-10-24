//! Changing how an existing account reaches its servers: the host, port, security and user
//! name the account's page in Settings edits.
//!
//! The new plan is tried before it is kept, by the same sign-in Add Account tries
//! ([`crate::sync::check_sign_in`]), so a typo in a host name is said at the form in the
//! server's words rather than found at the next sync as an account that stopped working. The
//! check reads the plan from the store, so the new one is written first and the old one put
//! back when the server refuses it or cannot be reached.

use crate::error::CoreError;
use mail_domain::AccountPlan;
use mail_runtime::AccountSecrets;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// Reach `account` by `plan` from now on, once its incoming server has taken the sign-in.
///
/// An error leaves the account with the plan it had. What the check learned of a server that
/// took the sign-in (its capabilities and folders) is kept, as a first sync would keep it.
pub async fn replan(
    store: &Arc<SqliteStore>,
    account: AccountId,
    plan: &AccountPlan,
    secrets: Arc<dyn AccountSecrets>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    let stored = store
        .account(account.clone())?
        .ok_or_else(|| CoreError::UnknownAccount(account.to_string()))?;
    let had = stored.plan.map_err(|e| CoreError::PlanUnreadable {
        address: stored.address.clone(),
        why: crate::sync::why(&e),
    })?;
    if had == *plan {
        return Ok(());
    }
    store.set_account_plan(account.clone(), plan)?;
    let checked = crate::sync::check_sign_in(store, &stored.address, secrets, now).await;
    if checked.is_err() {
        store.set_account_plan(account, &had)?;
    }
    checked
}
