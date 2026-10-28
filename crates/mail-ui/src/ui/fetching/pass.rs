//! What runs one account's pass for the window: the seam a test replaces.
//!
//! The client's scheduler runs it off the thread that draws (`mail_client::fetch`); this is the
//! call it makes to the servers.

use mail_client::Client;
pub(in crate::ui) use mail_client::fetch::Pass;
use std::sync::Arc;

/// What runs a pass. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Passer(pub Pass);

impl Passer {
    /// The servers, through [`mail_client::SyncOps::run_due`].
    #[cfg(not(test))]
    pub(in crate::ui) fn server(client: &Client) -> Self {
        Self(client.net().pass())
    }

    /// A pass that ends at once having fetched nothing: a window that must reach no server.
    pub(in crate::ui) fn idle() -> Self {
        Self(Arc::new(|_, _, _, _| Ok(Vec::new())))
    }

    /// Not in a test build: a pass reads the keyring and opens sockets, and a test that forgot
    /// to provide its own must fail in words rather than reach either.
    #[cfg(test)]
    pub(in crate::ui) fn server(_: &Client) -> Self {
        Self(Arc::new(|_, _, _, _| {
            Err(
                "a test ran a pass without providing a Passer; the real one reaches the servers"
                    .to_owned(),
            )
        }))
    }
}
