//! Push for an account that reads through Microsoft Graph: the relay it is pushed through, as a
//! setting of the account's.
//!
//! Graph is polled ([`mail_domain::presets::GRAPH_POLL_EVERY`]) unless the account names a relay
//! (`relay/` in the repository, deployed by whoever wants one). With one set, the account is
//! pushed to ([`super::live::pushing`]): its watch registers with the relay, subscribes at Graph
//! and holds a WebSocket to the relay (`mail_runtime::graph::push`), and each alert is a pass.
//! With none, or with the relay unreachable, nothing changes from polling: a watch that cannot
//! connect is lost, and the scheduler polls while it reconnects with its backoff.

use super::Configured;
use crate::error::{CoreError, Logged};
use mail_domain::Incoming;
use mail_runtime::graph::push::Relay;
use mail_store::SqliteStore;
use porter_core::AccountId;

/// The relay `account` is pushed through, as it was set. `None` when it is polled.
pub fn relay(store: &SqliteStore, account: AccountId) -> Option<String> {
    store
        .graph_push(account)
        .or_log("the account's push relay could not be read")
        .and_then(|held| held.relay)
}

/// Push `account` through the relay at `text`, or stop with an empty `text`. The address is
/// checked first (`https://`, or `http://` on this computer's loopback) and stored as it will
/// be used; what was registered with an earlier relay is let go by the next watch. Returns what
/// was stored.
pub fn set_relay(
    store: &SqliteStore,
    account: AccountId,
    text: &str,
) -> Result<Option<String>, CoreError> {
    let text = text.trim();
    if text.is_empty() {
        store.set_push_relay(account, None)?;
        return Ok(None);
    }
    let relay = Relay::parse(text)?;
    store.set_push_relay(account, Some(relay.as_str()))?;
    Ok(Some(relay.as_str().to_owned()))
}

/// Whether `account` reads through Graph and names a relay to be pushed through.
pub fn relayed(store: &SqliteStore, account: &Configured) -> bool {
    account.plan.incoming == Incoming::Graph && relay(store, account.id.clone()).is_some()
}
