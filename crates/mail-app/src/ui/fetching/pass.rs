//! One account's pass, run off the thread that draws, and what it sends back.

use super::Note;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::fetch::{Event, FolderFetch};
use mail_core::sync::report::{Hooks, PassEnd, Progress, outcome};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedSender, watch};

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

/// Which pass, or wait, of an account is the current one. A pass that was cancelled, or a wait
/// that a later one replaced, finds its number out of date and says nothing.
#[derive(Clone, Default)]
pub(super) struct Generations(Rc<RefCell<BTreeMap<AccountId, u64>>>);

impl Generations {
    /// A new one begins: every earlier one is out of date.
    pub fn next(&self, account: AccountId) -> u64 {
        let mut all = self.0.borrow_mut();
        let slot = all.entry(account).or_insert(0);
        *slot += 1;
        *slot
    }

    /// Whether `generation` is still the account's latest.
    pub fn current(&self, account: AccountId, generation: u64) -> bool {
        self.0.borrow().get(&account) == Some(&generation)
    }

    /// The account is gone.
    pub fn forget(&self, account: AccountId) {
        self.0.borrow_mut().remove(&account);
    }
}

/// What a pass needs that the runner owns.
pub(super) struct Running {
    pub account: AccountId,
    pub generation: u64,
    pub generations: Generations,
    pub tx: UnboundedSender<Note>,
    pub store: Arc<SqliteStore>,
    pub passer: Passer,
    /// Held for the whole pass: the engines read a dropped sender as a cancellation.
    pub cancel: Arc<watch::Sender<bool>>,
    pub folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    pub revision: Signal<u64>,
}

/// Whether a folder of `account` is being fetched on demand.
fn folder_in_flight(
    folders: &BTreeMap<(AccountId, String), FolderFetch>,
    account: AccountId,
) -> bool {
    folders
        .iter()
        .any(|((one, _), fetch)| *one == account && *fetch == FolderFetch::Fetching)
}

/// How often a pass waiting on a folder fetch looks again.
const POLITE: Duration = Duration::from_millis(200);

/// Run the pass, sending its progress and then its end to the runner.
///
/// Waits for a folder of the same account that is being fetched, because the two write the same
/// rows.
pub(super) async fn run(mut running: Running) {
    while folder_in_flight(&running.folders.peek(), running.account.clone()) {
        tokio::time::sleep(POLITE).await;
    }
    let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel::<Progress>();
    let (account, started) = (running.account, crate::ui::clock::now());
    let (store, passer, cancel) = (
        running.store.clone(),
        running.passer.clone(),
        running.cancel.subscribe(),
    );
    let this_account = account.clone();
    let blocking = async move {
        // `spawn_blocking`, not this task: a pass waits on the application's runtime
        // (`edge::block_on`), and `Runtime::block_on` inside an async context panics.
        let done = tokio::task::spawn_blocking(move || {
            let progress = |_: AccountId, progress: Progress| {
                let _ = ptx.send(progress);
            };
            let hooks = Hooks {
                progress: Some(&progress),
                cancel: BTreeMap::from([(this_account.clone(), cancel)]),
                ..Hooks::default()
            };
            (passer.0)(store, started, this_account, hooks)
        })
        .await;
        done.unwrap_or_else(|e| Err(format!("the sync pass stopped: {e}")))
    };
    let (generations, generation, tx) = (&running.generations, running.generation, &running.tx);
    let forward = async {
        // Ends when the pass drops its sender, which is when it has said everything.
        while let Some(progress) = prx.recv().await {
            if generations.current(account.clone(), generation) {
                let _ = tx.send(Note::Event(account.clone(), progress.event()));
            }
        }
    };
    let (done, ()) = futures_util::future::join(blocking, forward).await;
    let stored = may_have_stored(&done);
    let event: Event = outcome(done, account.clone());
    if generations.current(account.clone(), generation) {
        let _ = tx.send(Note::Event(account, event));
    }
    // What a pass stored is what the list reads, cancelled or not. A pass that stored nothing
    // leaves nothing to read again: a refused or unreachable account must not redraw the window.
    if stored {
        running.revision += 1;
    }
}

/// Whether a pass that ended like this may have written something the window shows.
///
/// A pass that ran, or was cancelled part-way, may have: counts do not say everything it
/// writes (flags are not counted). One that never got to run, or whose whole run failed, has not.
fn may_have_stored(done: &Result<Vec<PassEnd>, String>) -> bool {
    done.as_ref()
        .is_ok_and(|ends| ends.iter().any(PassEnd::may_have_stored))
}

#[cfg(test)]
mod tests;
