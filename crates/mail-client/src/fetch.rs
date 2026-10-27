//! Fetching mail: every account's [`Link`], and the one scheduler that moves them.
//!
//! The scheduler is `mail_core::schedule`, the loop `mailo watch` runs too. The client runs it
//! once, however many callers ask ([`crate::Client::fetch`]): on a thread of its own
//! (`mailo-fetch`) for a client with threads, so it outlives whoever started it; on the caller's
//! runtime for one that reads on its caller's thread ([`crate::Client::inline`]).
//!
//! What reaches a server is lent by the caller as [`Seams`]: the pass, the push, the folder
//! fetch, and who else schedules. The program lends the servers; a test lends its own.
//!
//! Where every account stands is one value, [`Fetched`], told to whoever follows it
//! ([`crate::Client::fetched`]). What it means to a person is the caller's.
//!
//! A send leaves with a pass, and a polled account's next pass may be minutes away: so when a
//! send queued for later (or held for Undo) comes due, its account is asked for a pass then
//! ([`outbox`]).

use crate::client::Shared;
use chrono::{DateTime, Utc};
use mail_core::fetch::{Event, FolderEffect, FolderEvent, FolderFetch, Link, Trigger};
use mail_core::schedule::{
    self, Change, Daemon, End, Heard, Hold, Host, ListenCall, Local, Lost, Note, PassCall, Passed,
    Schedule, Scheduler, Stop, Stored, Timing,
};
use mail_core::sync::report::{Hooks, Offer, PassEnd, Progress, outcome};
use mail_domain::{MailboxRef, Retry};
use mail_store::{SqliteStore, Store};
use parking_lot::Mutex;
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Run one account's pass: the signature of `mail_core::SyncOps::run_due`.
pub type Pass = Arc<
    dyn Fn(Arc<SqliteStore>, DateTime<Utc>, AccountId, Hooks<'_>) -> Result<Vec<PassEnd>, String>
        + Send
        + Sync,
>;

/// Hold one connection of a watch: the signature of `mail_core::SyncOps::listen`.
pub type Listen = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, Stop, Hold, &dyn Fn(Heard)) -> Result<(), Lost>
        + Send
        + Sync,
>;

/// Which accounts the server can push to: the signature of `mail_core::sync::live::pushing`.
pub type Pushers = Arc<dyn Fn(&SqliteStore) -> Vec<AccountId> + Send + Sync>;

/// Fetch one folder now: the signature of `mail_core::SyncOps::folder_now`.
pub type FetchFolder = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, &str, DateTime<Utc>) -> Result<PassEnd, String>
        + Send
        + Sync,
>;

/// Whether the notices speak, and whose new mail they say: the settings as they stand, handed to a
/// pass that runs on a thread of the client's, which cannot read what the caller's settings are.
#[derive(Debug, Clone, Copy)]
pub struct Speaking {
    pub on: bool,
    pub whom: mail_core::notify::Whom,
}

impl Default for Speaking {
    /// Notices on, for everyone's mail, as the setting's default is.
    fn default() -> Self {
        Speaking {
            on: true,
            whom: mail_core::notify::Whom::Everyone,
        }
    }
}

/// Where what a pass stored is announced, and the settings that say whether it is.
#[derive(Clone)]
pub struct Announce {
    pub notifier: Arc<dyn mail_core::notify::Notifier>,
    pub speaking: Arc<std::sync::Mutex<Speaking>>,
}

impl std::fmt::Debug for Announce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Announce(..)")
    }
}

/// Say what each finished pass in `ends` stored, as `mailo watch` does
/// (`mail_core::sync::watcher::told`). A message is an arrival only on the pass that first stores
/// it, so a watch and a window running side by side seldom both announce one.
fn announce(store: &SqliteStore, ends: &[PassEnd], on: &Announce, at: DateTime<Utc>) {
    let speaks = *on
        .speaking
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !speaks.on {
        return;
    }
    for end in ends {
        let PassEnd::Finished(report) = end else {
            continue;
        };
        if let Err(why) = mail_core::notify::announce(
            store,
            report.account.clone(),
            &report.counts.arrived,
            &report.counts.noted,
            on.notifier.as_ref(),
            speaks.whom,
            at,
        ) {
            log::warn!(
                "new mail for {} could not be announced: {why}",
                report.address
            );
        }
    }
}

/// What fetching reaches outside the client.
#[derive(Clone)]
pub struct Seams {
    pub pass: Pass,
    pub listen: Listen,
    pub pushers: Pushers,
    /// Where the person's settings of how often to check are kept, when they are.
    pub config: Option<std::path::PathBuf>,
    /// Where a pass that stored mail says so.
    pub announce: Option<Announce>,
    /// Whether the background daemon holds the one connection per account.
    pub daemon: Arc<dyn Fn() -> Daemon + Send + Sync>,
    /// Who schedules: this process, or a `mailo watch` beside it. Asked every time.
    pub schedule: Arc<dyn Fn() -> Schedule + Send + Sync>,
    pub folder: FetchFolder,
    pub timing: Timing,
}

impl std::fmt::Debug for Seams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seams")
            .field("timing", &self.timing)
            .finish_non_exhaustive()
    }
}

/// Where every account's fetching stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// Each account that has a server, and its link.
    pub links: BTreeMap<AccountId, Link>,
    /// What each account's last pass offered beside its failure ([`offer_of`]).
    pub offers: BTreeMap<AccountId, Offer>,
    /// Each folder fetched because it was opened, and where that stands.
    pub folders: BTreeMap<(AccountId, String), FolderFetch>,
}

impl Fetched {
    /// Where a folder's fetch on opening stands. A folder never opened is `Unfetched`.
    pub fn folder(&self, account: &AccountId, path: &str) -> FolderFetch {
        self.folders
            .get(&(account.clone(), path.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

/// The longest the outbox is left unread while nothing in it is due: what a send queued by
/// another process waits at most.
const OUTBOX_AT_MOST: Duration = Duration::from_secs(60);

/// How soon a send that came due while its account was fetching is asked for again: that pass
/// began before it was due, so it may have gone by the outbox already.
const OUTBOX_AGAIN: Duration = Duration::from_secs(1);

/// A folder to fetch, and when it was asked for.
type FolderAsk = (AccountId, String, DateTime<Utc>);

/// Fetching, as the client holds it once it was asked to fetch.
pub(crate) struct Fetcher {
    seams: Seams,
    tx: UnboundedSender<Note>,
    folders_tx: UnboundedSender<FolderAsk>,
    /// What the scheduler and the folder fetches receive, until they are started.
    inboxes: Mutex<Option<(UnboundedReceiver<Note>, UnboundedReceiver<FolderAsk>)>>,
    state: tokio::sync::watch::Sender<Arc<Fetched>>,
    /// The accounts last told to the scheduler, with how often each wants a pass.
    every: Mutex<Vec<(AccountId, Duration)>>,
    /// The accounts last found pushed to: a relay set on an account's page changes this without
    /// changing any interval, and the scheduler reads the accounts again when told.
    pushed: Mutex<Vec<AccountId>>,
    /// Woken when the outbox changed.
    outbox: tokio::sync::Notify,
}

/// Which accounts there are to fetch, and how often each wants a pass.
fn intervals(seams: &Seams, store: &SqliteStore) -> Vec<(AccountId, Duration)> {
    match &seams.config {
        Some(dir) => mail_core::sync::due::intervals_with(store, &mail_core::offline::load(dir)),
        None => mail_core::sync::due::intervals(store),
    }
}

impl Fetcher {
    pub(crate) fn new(seams: Seams, store: &SqliteStore, now: DateTime<Utc>) -> Fetcher {
        let (tx, inbox) = unbounded_channel();
        let (folders_tx, folders) = unbounded_channel();
        let every = intervals(&seams, store);
        let fetched = Fetched {
            links: schedule::initial(store, &every, now),
            ..Fetched::default()
        };
        // What the scheduler hears first: the accounts are known, and which to watch follows.
        // The receiver is held below, so the send is not refused.
        let _ = tx.send(Note::Accounts(every.clone()));
        let pushed = (seams.pushers)(store);
        Fetcher {
            seams,
            tx,
            folders_tx,
            inboxes: Mutex::new(Some((inbox, folders))),
            state: tokio::sync::watch::Sender::new(Arc::new(fetched)),
            pushed: Mutex::new(pushed),
            every: Mutex::new(every),
            outbox: tokio::sync::Notify::new(),
        }
    }

    /// The outbox changed: a send may have been queued, or have left.
    pub(crate) fn outbox_moved(&self) {
        self.outbox.notify_one();
    }

    pub(crate) fn fetched(&self) -> tokio::sync::watch::Receiver<Arc<Fetched>> {
        self.state.subscribe()
    }

    pub(crate) fn tell(&self, account: AccountId, event: Event) {
        // Fails only once the scheduler has stopped, and then nobody is left to tell.
        let _ = self.tx.send(Note::Event(account, event));
    }

    fn change(&self, edit: impl FnOnce(&mut Fetched)) {
        self.state
            .send_modify(|fetched| edit(Arc::make_mut(fetched)));
    }

    /// The accounts may have changed: tell the scheduler when they have.
    pub(crate) fn accounts(&self, store: &SqliteStore) {
        let now = intervals(&self.seams, store);
        let pushed_now = (self.seams.pushers)(store);
        let mut every = self.every.lock();
        let mut pushed = self.pushed.lock();
        if *every != now || *pushed != pushed_now {
            every.clone_from(&now);
            pushed.clone_from(&pushed_now);
            // As `tell`.
            let _ = self.tx.send(Note::Accounts(now));
        }
    }

    /// A folder's place was chosen: fetch it, unless that was done a moment ago or a pass is
    /// running on its account, which writes the same rows.
    pub(crate) fn open_folder(&self, mailbox: MailboxRef, now: DateTime<Utc>) {
        let MailboxRef { account, path } = mailbox;
        let before = self.state.borrow().clone();
        if before.links.get(&account).is_none_or(Link::is_busy) {
            return;
        }
        let (next, effect) = before.folder(&account, &path).step(FolderEvent::Open, now);
        self.change(|fetched| {
            fetched
                .folders
                .insert((account.clone(), path.clone()), next);
        });
        if effect == Some(FolderEffect::Fetch) {
            // As `tell`.
            let _ = self.folders_tx.send((account, path, now));
        }
    }

    /// Every folder fetched on opening is fetched again the next time it is opened.
    pub(crate) fn forget_folders(&self, now: DateTime<Utc>) {
        self.change(|fetched| {
            for folder in fetched.folders.values_mut() {
                *folder = folder.step(FolderEvent::Forget, now).0;
            }
        });
    }
}

/// What a finished fetch on opening says to its folder. A refused sign-in, or a fetch that never
/// ran, is a refusal in the server's words; a fetch that ran is done, whatever else went wrong,
/// since a folder partly up to date is still one to read. One cancelled is refused with nothing
/// said, so the next opening fetches it.
pub fn ended(done: Result<PassEnd, String>) -> FolderEvent {
    match done {
        Err(why) | Ok(PassEnd::Failed { why, .. }) => FolderEvent::Refused(why),
        Ok(PassEnd::Cancelled { .. }) => FolderEvent::Refused(String::new()),
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

/// What a run offered `account`: the [`Offer`] its pass failed with, if it failed with one.
///
/// Kept beside the link rather than in it, because the link is the scheduler's and an offer
/// says nothing about when to try again. Every pass's end replaces it.
pub fn offer_of(done: &Result<Vec<PassEnd>, String>, account: &AccountId) -> Option<Offer> {
    done.as_ref().ok()?.iter().find_map(|end| match end {
        PassEnd::Failed {
            account: failed,
            offer,
            ..
        } if failed == account => *offer,
        _ => None,
    })
}

/// Whether a pass that ended like this may have written something.
///
/// A pass that ran, or was cancelled part-way, may have: counts do not say everything it writes
/// (flags are not counted). One that never got to run, or whose whole run failed, has not.
pub fn may_have_stored(done: &Result<Vec<PassEnd>, String>) -> bool {
    done.as_ref()
        .is_ok_and(|ends| ends.iter().any(PassEnd::may_have_stored))
}

/// The client, as the scheduler's host.
struct ClientHost {
    shared: Arc<Shared>,
}

impl ClientHost {
    fn fetcher(&self) -> &Fetcher {
        self.shared
            .fetcher()
            .expect("the scheduler runs only once fetching was asked for")
    }
}

impl Host for ClientHost {
    fn now(&self) -> DateTime<Utc> {
        self.shared.now()
    }

    fn schedule(&self) -> Schedule {
        (self.fetcher().seams.schedule)()
    }

    fn daemon(&self) -> Daemon {
        (self.fetcher().seams.daemon)()
    }

    fn pushers(&self) -> Vec<AccountId> {
        (self.fetcher().seams.pushers)(self.shared.store())
    }

    fn folder_in_flight(&self, account: &AccountId) -> bool {
        self.fetcher()
            .state
            .borrow()
            .folders
            .iter()
            .any(|((one, _), fetch)| one == account && *fetch == FolderFetch::Fetching)
    }

    fn timing(&self) -> Timing {
        self.fetcher().seams.timing
    }

    fn pass(&self, call: PassCall) -> Local<'_, Passed> {
        let store = self.shared.store_arc();
        let pass = self.fetcher().seams.pass.clone();
        let announcing = self.fetcher().seams.announce.clone();
        Box::pin(async move {
            let PassCall {
                account,
                started,
                cancel,
                progress,
            } = call;
            let this_account = account.clone();
            // A thread of its own: a pass waits on the network, and may wait on a runtime of its
            // own, which an async task must not.
            let done = tokio::task::spawn_blocking(move || {
                let said = |_: AccountId, step: Progress| {
                    // The scheduler stops listening only once it is gone.
                    let _ = progress.send(step.event());
                };
                let hooks = Hooks {
                    progress: Some(&said),
                    cancel: BTreeMap::from([(this_account.clone(), cancel)]),
                };
                let done = pass(store.clone(), started, this_account, hooks);
                if let (Some(on), Ok(ends)) = (&announcing, &done) {
                    announce(&store, ends, on, started);
                }
                done
            })
            .await
            .unwrap_or_else(|e| Err(format!("the sync pass stopped: {e}")));
            let offer = offer_of(&done, &account);
            if self.fetcher().state.borrow().offers.get(&account) != offer.as_ref() {
                self.fetcher().change(|fetched| match offer {
                    Some(offer) => {
                        fetched.offers.insert(account.clone(), offer);
                    }
                    None => {
                        fetched.offers.remove(&account);
                    }
                });
            }
            let stored = if may_have_stored(&done) {
                Stored::Maybe
            } else {
                Stored::Nothing
            };
            Passed {
                event: outcome(done, account),
                stored,
            }
        })
    }

    fn listen(&self, call: ListenCall) -> Local<'_, Result<(), Lost>> {
        let store = self.shared.store_arc();
        let listen = self.fetcher().seams.listen.clone();
        Box::pin(async move {
            let ListenCall {
                account,
                stop,
                hold,
                heard,
            } = call;
            // A thread of its own: the watch parks in a socket.
            tokio::task::spawn_blocking(move || {
                let say = |said: Heard| heard(said);
                listen(store, account, stop, hold, &say)
            })
            .await
            .unwrap_or_else(|e| {
                Err(Lost {
                    retry: Retry::Fatal(e.to_string()),
                    why: format!("the watch stopped: {e}"),
                })
            })
        })
    }

    fn changed(&self, change: Change) {
        match change {
            Change::Link(account, link) => self.fetcher().change(|fetched| {
                fetched.links.insert(account, link);
            }),
            Change::Gone(account) => self.fetcher().change(|fetched| {
                fetched.links.remove(&account);
                fetched.offers.remove(&account);
                fetched.folders.retain(|(one, _), _| *one != account);
            }),
            // What a pass stored reaches the client as the store's own commits.
            Change::Stored => self.shared.stored(),
        }
    }
}

/// Fetching for `shared`, run until it is dropped: the scheduler, and the folders fetched on
/// opening. `None` once it has been started already.
pub(crate) fn run(shared: Arc<Shared>) -> Option<impl std::future::Future<Output = ()>> {
    let fetcher = shared.fetcher()?;
    let (inbox, asked) = fetcher.inboxes.lock().take()?;
    let links = fetcher.state.borrow().links.clone();
    let every = fetcher.every.lock().iter().cloned().collect();
    let tx = fetcher.tx.clone();
    Some(async move {
        let host = ClientHost {
            shared: shared.clone(),
        };
        let scheduler = Scheduler::new(&host, shared.store_arc(), links, every, tx);
        let ((), (), _) = tokio::join!(
            folders(shared.clone(), asked),
            outbox(shared.clone()),
            scheduler.run(inbox, End::Never)
        );
    })
}

/// Fetch each folder asked for, each on a thread of its own, and say how it went.
async fn folders(shared: Arc<Shared>, mut asked: UnboundedReceiver<FolderAsk>) {
    while let Some((account, path, now)) = asked.recv().await {
        let shared = shared.clone();
        tokio::spawn(async move {
            let Some(fetcher) = shared.fetcher() else {
                return;
            };
            let (fetch, store) = (fetcher.seams.folder.clone(), shared.store_arc());
            let (one, at) = (account.clone(), path.clone());
            let done = tokio::task::spawn_blocking(move || fetch(store, one, &at, now))
                .await
                .unwrap_or_else(|e| Err(format!("fetching stopped: {e}")));
            let (event, at) = (ended(done), shared.now());
            fetcher.change(|fetched| {
                let folder = fetched.folders.entry((account, path)).or_default();
                *folder = folder.step(event, at).0;
            });
            shared.stored();
        });
    }
}

/// Ask each account for a pass when a send in its outbox comes due: one queued for later, or
/// held for Undo. Sends already due when fetching began go with the first pass.
async fn outbox(shared: Arc<Shared>) {
    let Some(fetcher) = shared.fetcher() else {
        return;
    };
    // Per account, the moment up to which its sends have been asked for.
    let mut asked: BTreeMap<AccountId, DateTime<Utc>> = BTreeMap::new();
    let began = shared.now();
    loop {
        let now = shared.now();
        let accounts: Vec<AccountId> = fetcher
            .every
            .lock()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        let mut wait = OUTBOX_AT_MOST;
        for account in accounts {
            let after = *asked.entry(account.clone()).or_insert(began);
            // An outbox that cannot be read now is read again on the next look.
            let Ok(Some(due)) = shared.store().outbox_next(account.clone(), after) else {
                continue;
            };
            if due > now {
                wait = wait.min((due - now).to_std().unwrap_or_default());
                continue;
            }
            let busy = fetcher
                .state
                .borrow()
                .links
                .get(&account)
                .is_none_or(Link::is_busy);
            if busy {
                wait = wait.min(OUTBOX_AGAIN);
            } else {
                fetcher.tell(account.clone(), Event::Start(Trigger::Manual));
                asked.insert(account, due);
                wait = Duration::ZERO;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = fetcher.outbox.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests;
