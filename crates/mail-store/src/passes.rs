//! How an account's passes last ended (migration 0030): the freshness a window shows.

use chrono::{DateTime, Utc};

/// How an account's passes last ended, as the store recorded them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Passes {
    /// When a pass of the account last ran to its end, whatever trouble it met on the way.
    /// `None` until one has.
    pub finished: Option<DateTime<Utc>>,
    /// The failure that ended the latest pass, when the latest pass failed. Always newer than
    /// `finished`: a pass that finishes clears it.
    pub failed: Option<PassFailed>,
}

/// A pass that could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassFailed {
    pub at: DateTime<Utc>,
    /// What a person was told.
    pub why: String,
}
