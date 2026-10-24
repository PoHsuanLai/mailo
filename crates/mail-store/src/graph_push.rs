//! Push through a relay for an account that reads through Microsoft Graph (migration 0033).

use chrono::{DateTime, Utc};

/// What the store holds about one account's push: the relay it is set to, and what was set up
/// there and with Graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphPush {
    /// The relay's base URL, as the person set it. `None` is no push.
    pub relay: Option<String>,
    /// The key registered with a relay, and the relay it was registered with. A key belongs to
    /// one relay: set another, and a new one is registered there.
    pub registered: Option<Registered>,
    /// The subscription Graph sends that relay its notifications under. Only ever beside a key.
    pub subscription: Option<Subscribed>,
}

/// A key a relay knows this account by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub relay: String,
    pub key: String,
}

/// A subscription Graph holds for this account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribed {
    /// Graph's id for it.
    pub id: String,
    /// When Graph ends it, unless it is renewed first.
    pub expires: DateTime<Utc>,
}
