//! Push in the window: how the window holds a connection to hear the server.
//!
//! The scheduler (`mail_core::schedule`) decides which accounts are watched, turns what the
//! watch hears into the events the link reads, and reconnects when the connection is lost; what
//! it is lent is [`Listener`], the one place the window reaches a server to wait on it and the
//! lock that says whether anyone else already does. A test provides its own.

use mail_core::fetch::RETRY_NOW;
use mail_core::schedule::{Daemon, Heard, Hold, Lost, Stop};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

/// Runs one connection of a watch: the signature of [`mail_core::SyncOps::listen`], with the
/// grace chosen.
type Listen = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, Stop, Hold, &dyn Fn(Heard)) -> Result<(), Lost>
        + Send
        + Sync,
>;

/// Which accounts the server can push to: the signature of [`mail_core::sync::live::pushing`].
type Pushers = Arc<dyn Fn(&SqliteStore) -> Vec<AccountId> + Send + Sync>;

/// What the watch is, and what it asks of its surroundings. The real ones unless a test
/// provided its own, as [`super::Passer`] is.
#[derive(Clone)]
pub(in crate::ui) struct Listener {
    pub(super) listen: Listen,
    /// Every account that can be pushed to, as the store describes them now.
    pub(super) pushers: Pushers,
    pub(super) daemon: Arc<dyn Fn() -> Daemon + Send + Sync>,
    /// How long after being wanted a watch connects, so that opening a window is not also a
    /// second connection racing the first pass.
    pub(super) first: Duration,
    /// The floor under the wait between one lost connection and the next attempt.
    pub(super) retry: Duration,
}

impl Listener {
    /// The servers, through [`mail_core::SyncOps::listen`]; the daemon, through its lock.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self {
            listen: Arc::new(|store, account, stop, hold, heard| {
                let mail = crate::edge::mail(&store);
                crate::edge::block_on(mail.sync().listen(
                    account,
                    stop,
                    Some(hold),
                    mail_core::sync::live::GRACE,
                    heard,
                ))
            }),
            pushers: Arc::new(mail_core::sync::live::pushing),
            daemon: Arc::new(|| {
                // Racy by nature, as the lock's own documentation says: a daemon that starts a
                // moment later is noticed the next time a link rests or the accounts change.
                match mail_core::ipc::agent() {
                    Ok(agent) if agent.is_running() => Daemon::Holding,
                    // A `mailo watch` is the one that holds the connection, and the window
                    // leaves scheduled fetching to it (`super::delegate`).
                    _ if mail_core::ipc::watching::running() => Daemon::Holding,
                    _ => Daemon::Absent,
                }
            }),
            first: Duration::from_secs(4),
            retry: RETRY_NOW,
        }
    }

    /// Not in a test build: a watch opens sockets, and a test that forgot to provide its own
    /// must not reach one. No account is pushed to, so none is watched.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self {
            listen: Arc::new(|_, _, _, _, _| {
                Err(Lost {
                    retry: mail_domain::Retry::Fatal(
                        "a test watched without providing a Listener".to_owned(),
                    ),
                    why: "a test watched without providing a Listener".to_owned(),
                })
            }),
            pushers: Arc::new(|_| Vec::new()),
            daemon: Arc::new(|| Daemon::Absent),
            first: Duration::ZERO,
            retry: RETRY_NOW,
        }
    }
}

#[cfg(test)]
mod tests;
