//! Opening a server folder's place: what the list's title says of it, and fetching it.
//!
//! A pass fetches the folders the user follows; one opened from "Show all" is fetched by nobody
//! until it is opened. So choosing a folder fetches it now, off the thread that draws, with
//! the list's own busy line while it runs — once per choice, and not again for the same folder
//! within [`again`] unless Sync is pressed. What fetches is a context, [`Fetcher`], so a test
//! can count what the window asks for without a server.

use crate::sync::Ran;
use crate::view::{Shell, SyncState, folder_of, synced};
use chrono::{DateTime, TimeDelta, Utc};
use dioxus::prelude::*;
use mail_domain::{AccountId, MailboxRef};
use mail_store::SqliteStore;
use std::sync::Arc;

/// Fetch one folder now: the signature of [`crate::sync::folder_now`].
pub(in crate::ui) type Fetch = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, &str, DateTime<Utc>) -> Result<Ran, String> + Send + Sync,
>;

/// What fetches a folder. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Fetcher(pub Fetch);

impl Fetcher {
    /// The server, through [`crate::sync::folder_now`].
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|store, account, path, now| {
            crate::sync::folder_now(store, account, path, now)
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

/// How long an opened folder is left alone before opening it fetches again.
pub(in crate::ui) fn again() -> TimeDelta {
    TimeDelta::seconds(60)
}

/// When each folder was last fetched on being opened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::ui) struct Recent(Vec<(MailboxRef, DateTime<Utc>)>);

impl Recent {
    /// Whether opening `mailbox` at `now` fetches it: not if it was fetched within [`again`].
    pub(in crate::ui) fn due(&self, mailbox: &MailboxRef, now: DateTime<Utc>) -> bool {
        !self
            .0
            .iter()
            .any(|(one, at)| one == mailbox && *at <= now && now - *at < again())
    }

    /// `mailbox` was fetched at `now`.
    pub(in crate::ui) fn mark(&mut self, mailbox: MailboxRef, now: DateTime<Utc>) {
        self.0.retain(|(one, _)| *one != mailbox);
        self.0.push((mailbox, now));
    }

    /// Sync was pressed: every folder fetches again on its next opening.
    pub(in crate::ui) fn forget(&mut self) {
        self.0.clear();
    }
}

/// What the window shares for fetching on opening: the list's busy line and what was fetched.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Fetching {
    pub state: Signal<SyncState>,
    pub recent: Signal<Recent>,
}

/// Provide [`Fetching`] for the window, over the sync state the list bar shows. Once, from `App`.
pub(in crate::ui) fn use_fetching(state: Signal<SyncState>) -> Fetching {
    use_context_provider(|| Fetching {
        state,
        recent: Signal::new(Recent::default()),
    })
}

/// A folder's place was chosen: fetch it, unless that was done a moment ago or a pass is
/// already running, then read the list and the badges again.
///
/// Called from the click that chose it, which is where a spawned task is polled (F140).
pub(in crate::ui) fn opened(mailbox: MailboxRef, mut revision: Signal<u64>) {
    let Some(Fetching {
        mut state,
        mut recent,
    }) = try_consume_context::<Fetching>()
    else {
        return;
    };
    let now = Utc::now();
    if !recent.peek().due(&mailbox, now) || !state.peek().may_start() {
        return;
    }
    recent.write().mark(mailbox.clone(), now);
    state.set(SyncState::Running);
    let fetch = try_consume_context::<Fetcher>().unwrap_or_else(Fetcher::server);
    let store = consume_context::<Arc<SqliteStore>>();
    spawn(async move {
        let MailboxRef { account, path } = mailbox;
        let named = path.clone();
        // `spawn_blocking`: the fetch opens a socket on a runtime of its own.
        let done = tokio::task::spawn_blocking(move || (fetch.0)(store, account, &path, now)).await;
        state.set(match done {
            Ok(result) => synced(
                result
                    .map(|ran| ran.text)
                    .map_err(|why| format!("{named} was not fetched: {why}")),
            ),
            Err(e) => synced(Err(format!("fetching {named} stopped: {e}"))),
        });
        revision += 1;
    });
}

/// Sync was pressed: the next opening of any folder fetches it again.
pub(in crate::ui) fn forget() {
    if let Some(Fetching { mut recent, .. }) = try_consume_context::<Fetching>() {
        recent.write().forget();
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
    let several = match shell.scope.as_slice() {
        [] => accounts.len() > 1,
        [_] => false,
        _ => true,
    };
    several
        .then(|| accounts.iter().find(|(id, _)| *id == mailbox.account))
        .flatten()
        .map(|(_, address)| address.clone())
}

#[cfg(test)]
mod tests;
