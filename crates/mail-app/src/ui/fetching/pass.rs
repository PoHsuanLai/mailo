//! What runs one account's pass for the window: the seam a test replaces.
//!
//! The pass is run off the thread that draws, by [`super::host`], which hands the scheduler how it
//! ended; this is the call it makes to the servers.

use chrono::{DateTime, Utc};
use mail_core::sync::report::{Hooks, PassEnd};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// Run a pass over one account: the signature of [`mail_core::SyncOps::run_due`].
pub(in crate::ui) type Pass = Arc<
    dyn Fn(Arc<SqliteStore>, DateTime<Utc>, AccountId, Hooks<'_>) -> Result<Vec<PassEnd>, String>
        + Send
        + Sync,
>;

/// What runs a pass. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Passer(pub Pass);

impl Passer {
    /// The servers, through [`mail_core::SyncOps::run_due`].
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|store, _now, account, hooks| {
            let mail = crate::edge::mail(&store);
            crate::edge::block_on(mail.sync().run_due(&[account], hooks)).map_err(String::from)
        }))
    }

    /// Not in a test build: a pass reads the keyring and opens sockets, and a test that forgot
    /// to provide its own must fail in words rather than reach either.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|_, _, _, _| {
            Err(
                "a test ran a pass without providing a Passer; the real one reaches the servers"
                    .to_owned(),
            )
        }))
    }
}

/// Whether a pass that ended like this may have written something the window shows.
///
/// A pass that ran, or was cancelled part-way, may have: counts do not say everything it
/// writes (flags are not counted). One that never got to run, or whose whole run failed, has not.
pub(super) fn may_have_stored(done: &Result<Vec<PassEnd>, String>) -> bool {
    done.as_ref()
        .is_ok_and(|ends| ends.iter().any(PassEnd::may_have_stored))
}

#[cfg(test)]
mod tests;
