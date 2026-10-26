//! Opening a server folder's place: what fetches it.
//!
//! A pass fetches the folders the user follows; one opened from "Show all" is fetched by nobody
//! until it is opened. So choosing a folder fetches it now (`fetching::Fetching::open_folder`
//! decides when, and keeps where each folder stands). What does the fetching is a context,
//! [`Fetcher`], so a test can count what the window asks for without a server.

use mail_client::Client;
/// Fetch one folder now: the signature of [`mail_client::SyncOps::folder_now`].
pub(in crate::ui) use mail_client::fetch::FetchFolder as Fetch;
use std::sync::Arc;

/// What fetches a folder. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Fetcher(pub Fetch);

impl Fetcher {
    /// The server, through [`mail_client::SyncOps::folder_now`].
    #[cfg(not(test))]
    pub(in crate::ui) fn server(client: &Client) -> Self {
        let client = client.clone();
        // The client's own store, which is the one it lends.
        Self(Arc::new(move |_, account, path, _now| {
            client
                .net()
                .folder_now(account, path)
                .map_err(crate::ui::remedy::told)
        }))
    }

    /// Not in a test build: `folder_now` reads the keyring and opens a socket, and a test that
    /// forgot to provide its own fetcher must fail in words rather than reach either.
    #[cfg(test)]
    pub(in crate::ui) fn server(_: &Client) -> Self {
        Self(Arc::new(|_, _, path, _| {
            Err(format!(
                "a test opened {path} without providing a Fetcher; the real one reaches the server"
            ))
        }))
    }
}

#[cfg(test)]
mod tests;
