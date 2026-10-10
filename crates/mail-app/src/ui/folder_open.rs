//! Opening a server folder's place: what the list's title says of it, and what fetches it.
//!
//! A pass fetches the folders the user follows; one opened from "Show all" is fetched by nobody
//! until it is opened. So choosing a folder fetches it now (`fetching::Fetching::open_folder`
//! decides when, and keeps where each folder stands). What does the fetching is a context,
//! [`Fetcher`], so a test can count what the window asks for without a server.

use crate::ui::view::{Shell, folder_of};
use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::sync::report::PassEnd;
use porter_core::AccountId;
use std::sync::Arc;

/// Fetch one folder now: the signature of [`mail_core::SyncOps::folder_now`].
pub(in crate::ui) type Fetch = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, &str, DateTime<Utc>) -> Result<PassEnd, String>
        + Send
        + Sync,
>;

/// What fetches a folder. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Fetcher(pub Fetch);

impl Fetcher {
    /// The server, through [`mail_core::SyncOps::folder_now`].
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|store, account, path, _now| {
            let mail = crate::edge::mail(&store);
            crate::edge::block_on(mail.sync().folder_now(account, path))
                .map_err(crate::ui::remedy::told)
        }))
    }

    /// Not in a test build: `folder_now` reads the keyring and opens a socket, and a test that
    /// forgot to provide its own fetcher must fail in words rather than reach either.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|_, _, path, _| {
            Err(format!(
                "a test opened {path} without providing a Fetcher; the real one reaches the server"
            ))
        }))
    }
}

/// The address the list's title names beside a folder, when the folder's account is one of
/// several in view. A pressed tile names its own address already.
pub(in crate::ui) fn title_address(
    shell: &Shell,
    accounts: &[(AccountId, String)],
) -> Option<String> {
    if shell.account.is_some() {
        return None;
    }
    let mailbox = folder_of(shell.places.get(shell.selected)?)?;
    let several = match &shell.scope {
        crate::ui::space::Scope::All => accounts.len() > 1,
        crate::ui::space::Scope::Accounts(ids) => ids.len() > 1,
    };
    several
        .then(|| accounts.iter().find(|(id, _)| *id == mailbox.account))
        .flatten()
        .map(|(_, address)| address.clone())
}

#[cfg(test)]
mod tests;
