//! How a failure should be handled. Without this split the outbox backoff loop has nothing
//! to branch on, and "reconnect this account" has no way to reach the UI.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// What to do about a failed operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Retry {
    /// Transient and cheap to retry at once, e.g. a dropped connection mid-command.
    Now,
    /// Retry after a delay. The runtime applies its own backoff on top of this floor.
    After(Duration),
    /// Credentials were rejected or a token was revoked. Retrying cannot help; the user
    /// must reauthenticate. Surfaces in the UI as a per-account banner.
    NeedsReauth,
    /// Permanently failed. The queued operation is dropped and its undo patch applied.
    Fatal(String),
}

/// Implemented by every error type in the workspace, so a failure can be routed without the
/// caller matching on a crate-specific enum.
pub trait Retryable {
    /// How this failure should be handled.
    fn retry(&self) -> Retry;
}
