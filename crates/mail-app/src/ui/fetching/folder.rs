//! Fetching a folder because it was opened.

use super::Fetching;
use crate::ui::folder_open::Fetcher;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::fetch::{Event, FolderEffect, FolderEvent, FolderFetch};
use mail_core::sync::report::PassEnd;
use mail_domain::{MailboxRef, Retry};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// What a finished on-demand fetch says to its folder.
///
/// A refused sign-in is a refusal: the folder was not fetched, and what the server said is the
/// reason. Trouble of any other kind leaves the folder as fetched as it got: the pass says what
/// went wrong, and a folder that is only partly up to date is still one to read.
pub(super) fn ended(done: Result<PassEnd, String>, named: &str) -> FolderEvent {
    match done {
        Err(why) | Ok(PassEnd::Failed { why, .. }) => {
            FolderEvent::Refused(format!("{named} was not fetched: {why}"))
        }
        Ok(PassEnd::Cancelled { .. }) => {
            FolderEvent::Refused(format!("{named} was not fetched: it was cancelled"))
        }
        Ok(end @ PassEnd::Finished(_)) => match end.event() {
            Event::Failed {
                retry: Retry::NeedsReauth,
                why,
                ..
            } => FolderEvent::Refused(why),
            _ => FolderEvent::Done,
        },
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
        if self.link(account.clone()).is_none_or(|link| link.is_busy()) {
            return;
        }
        let key = (account.clone(), path.clone());
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
            let fetching = account.clone();
            // `spawn_blocking`: the fetch waits on the application's runtime (`edge::block_on`), which
            // an async task must not.
            let done = tokio::task::spawn_blocking(move || (fetch.0)(store, fetching, &path, now))
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

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::fetch::Pause;
    use mail_core::sync::report::{AccountReport, Counts, Trouble};
    use mail_domain::id::account_id_from_uuid;
    use std::time::Duration;

    fn acct_account() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c9"))
    }

    fn finished(trouble: Vec<Trouble>) -> Result<PassEnd, String> {
        Ok(PassEnd::Finished(AccountReport {
            account: acct_account(),
            address: "me@nowhere.example".to_owned(),
            counts: Counts::default(),
            trouble,
        }))
    }

    fn trouble(retry: Retry, why: Option<&str>) -> Trouble {
        Trouble {
            mailbox: None,
            retry,
            why: why.map(str::to_owned),
        }
    }

    #[test]
    fn a_fetch_that_ran_is_done_even_with_trouble_that_is_not_a_refusal() {
        assert_eq!(ended(finished(Vec::new()), "Archive"), FolderEvent::Done);
        let slow = trouble(Retry::After(Duration::from_secs(60)), None);
        let odd = trouble(Retry::Now, Some("Archive: cannot select"));
        assert_eq!(
            ended(finished(vec![slow, odd]), "Archive"),
            FolderEvent::Done
        );
    }

    #[test]
    fn a_refused_sign_in_is_the_folders_refusal_in_the_servers_words() {
        let refused = trouble(Retry::NeedsReauth, Some("login failed"));
        assert_eq!(
            ended(finished(vec![refused]), "Archive"),
            FolderEvent::Refused("login failed".to_owned())
        );
    }

    #[test]
    fn a_fetch_that_never_ran_says_which_folder_was_not_fetched() {
        let failed = Ok(PassEnd::Failed {
            account: acct_account(),
            address: "me@nowhere.example".to_owned(),
            retry: Retry::Now,
            why: "cannot connect".to_owned(),
            pause: Pause::Unreachable,
        });
        let said = FolderEvent::Refused("Archive was not fetched: cannot connect".to_owned());
        assert_eq!(ended(failed, "Archive"), said);
        assert_eq!(
            ended(Err("cannot connect".to_owned()), "Archive"),
            said,
            "a request refused before it began reads the same"
        );
    }
}
