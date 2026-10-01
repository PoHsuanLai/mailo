//! Fetching a folder because it was opened.

use super::Fetching;
use crate::ui::folder_open::Fetcher;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::fetch::{FolderEffect, FolderEvent, FolderFetch};
use mail_domain::{AccountId, MailboxRef};
use mail_store::SqliteStore;
use std::sync::Arc;

/// What a finished on-demand fetch says to its folder.
///
/// A refused sign-in is a refusal: the folder was not fetched, and what it said is the reason.
pub(super) fn ended(done: Result<mail_core::sync::Ran, String>, named: &str) -> FolderEvent {
    match done {
        Ok(ran) if ran.rejected => FolderEvent::Refused(ran.text.trim_end().to_owned()),
        Ok(_) => FolderEvent::Done,
        Err(why) => FolderEvent::Refused(format!("{named} was not fetched: {why}")),
    }
}

impl Fetching {
    /// A folder's place was chosen: fetch it, unless that was done a moment ago or a pass is
    /// running on its account, then read the list and the badges again.
    ///
    /// Called from the click that chose it, which is where a spawned task is polled (F140).
    pub(in crate::ui) fn open_folder(&self, mailbox: MailboxRef, mut revision: Signal<u64>) {
        let MailboxRef { account, path } = mailbox;
        // Two writers on one account's rows: the pass has them, and opening goes without.
        if self.link(account).is_none_or(|link| link.is_busy()) {
            return;
        }
        let key = (account, path.clone());
        let now = Utc::now();
        let mut folders = self.folders;
        let (next, effect) = self.folder_state(&key).step(FolderEvent::Open, now);
        folders.write().insert(key.clone(), next);
        if effect != Some(FolderEffect::Fetch) {
            return;
        }
        let fetch = try_consume_context::<Fetcher>().unwrap_or_else(Fetcher::server);
        let store = consume_context::<Arc<SqliteStore>>();
        spawn(async move {
            let named = path.clone();
            // `spawn_blocking`: the fetch opens a socket on a runtime of its own.
            let done = tokio::task::spawn_blocking(move || (fetch.0)(store, account, &path, now))
                .await
                .unwrap_or_else(|e| Err(format!("fetching stopped: {e}")));
            settle(&mut folders, (account, named.clone()), ended(done, &named));
            revision += 1;
        });
    }

    /// Sync was pressed: the next opening of any folder fetches it again.
    pub(in crate::ui) fn forget_folders(&self) {
        let mut folders = self.folders;
        let kept: Vec<_> = folders.peek().keys().cloned().collect();
        for key in kept {
            settle(&mut folders, key, FolderEvent::Forget);
        }
    }

    /// Where a folder's on-demand fetch stands. A folder never opened is `Unfetched`.
    #[allow(dead_code)] // The sidebar's row of the next wave is its reader.
    pub(in crate::ui) fn folder(&self, account: AccountId, path: &str) -> FolderFetch {
        self.folders
            .read()
            .get(&(account, path.to_owned()))
            .cloned()
            .unwrap_or_default()
    }

    fn folder_state(&self, key: &(AccountId, String)) -> FolderFetch {
        self.folders.peek().get(key).cloned().unwrap_or_default()
    }
}

/// Move one folder by `event`, now.
fn settle(
    folders: &mut Signal<std::collections::BTreeMap<(AccountId, String), FolderFetch>>,
    key: (AccountId, String),
    event: FolderEvent,
) {
    let now: DateTime<Utc> = Utc::now();
    let current = folders.peek().get(&key).cloned().unwrap_or_default();
    let (next, _) = current.step(event, now);
    folders.write().insert(key, next);
}

/// A folder's place was chosen: [`Fetching::open_folder`], for the window that has one.
pub(in crate::ui) fn opened(mailbox: MailboxRef, revision: Signal<u64>) {
    if let Some(fetching) = try_consume_context::<Fetching>() {
        fetching.open_folder(mailbox, revision);
    }
}
