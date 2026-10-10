//! The one scheduler of mail fetching: every account's [`Link`], and the loop that moves them.
//!
//! The window and `mailo watch` fetch mail the same way, so they share this. Everything that
//! happens to an account arrives as a [`Note`]: a person's press, a pass's progress and end, a
//! timer's tick, the server's push. Each is given to [`fetch::step`], the next link is reported
//! to the [`Host`] as a [`Change`], and the effects the step asked for are run: a pass, a cancel,
//! a timer. The link is the state and this is the only thing that moves it, so there is no second
//! place where "is it syncing" is decided.
//!
//! What this crate does not know it is lent by the [`Host`]: how a pass is run (and how it is
//! told in words, which is the front end's), how a connection is held to hear the server push,
//! and what to do with a change. The scheduler spawns nothing and needs nothing to be `Send`:
//! it polls what it started on the task that runs [`Scheduler::run`], so a host may lend it
//! borrowed state, a signal or a notifier.
//!
//! Beside this, a push is a long-lived watch per account the server can push to ([`live`]); the
//! background daemon and a `mailo watch` are kept from running two ([`Daemon`], [`Schedule`]).

mod delegate;
mod live;
mod start;
mod tasks;

pub use delegate::{Schedule, Verdict, verdict};
pub use live::{
    Action, Daemon, Dirty, Heard, Hold, ListenCall, Lost, Push, Reconnect, Rerun, Stop, decide,
    event_for, reconnect,
};
pub use start::{Changes, initial, last_synced, link_for, probe, reconcile};
pub use tasks::Local;

use crate::fetch::{self, Effect, Event, Link, Live, Trigger};
use chrono::{DateTime, TimeDelta, Utc};
use live::{Keeping, Lives, keep};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tasks::{Generations, Task, Tasks, stuck};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

/// How soon a freshly opened window starts fetching, so that opening it is not also a network
/// round trip competing with the first paint.
pub const BEAT: Duration = Duration::from_secs(2);

/// How soon after being wanted a live watch connects, so that opening a window is not also a
/// second connection racing the first pass.
pub const FIRST: Duration = Duration::from_secs(4);

/// The interval an account is polled at when nothing says otherwise.
const FALLBACK: Duration = Duration::from_secs(300);

/// A timer is set for a little after the time it is for, so that it never wakes a link a
/// moment before the link would agree it is due.
const SLACK: Duration = Duration::from_millis(250);

/// The longest a single timer sleeps. A wake that is further off than this is asked for again.
const FAR: Duration = Duration::from_secs(24 * 3600);

/// How often a pass waiting on a folder fetch looks again.
const POLITE: Duration = Duration::from_millis(200);

/// What reaches the scheduler.
#[derive(Debug)]
pub enum Note {
    /// Something happened to an account's link.
    Event(AccountId, Event),
    /// The accounts that have a server, with how often each wants a pass, are these now.
    Accounts(Vec<(AccountId, Duration)>),
}

/// What the scheduler reports to its host, in the order it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// An account's link is this now.
    Link(AccountId, Link),
    /// The account is gone, with its link and anything running for it.
    Gone(AccountId),
    /// A pass may have written something the window shows, so a reader should look again. A pass
    /// that stored nothing says nothing: a refused or unreachable account must not redraw.
    Stored,
}

/// Whether a pass that ended may have written to the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Maybe,
    Nothing,
}

/// How a pass ended, as the link hears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passed {
    /// The one event the link hears when the pass is over.
    pub event: Event,
    pub stored: Stored,
}

/// What the host is asked to run a pass with.
#[derive(Debug)]
pub struct PassCall {
    pub account: AccountId,
    /// When the pass was started.
    pub started: DateTime<Utc>,
    /// Ends the pass: set, or its sender dropped.
    pub cancel: watch::Receiver<bool>,
    /// Where the pass says where it has got to. Dropped by the host when it has said everything.
    pub progress: UnboundedSender<Event>,
}

/// When the scheduler waits, as the host sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How soon the accounts already known first poll.
    pub beat: Duration,
    /// How soon after being wanted a live watch connects.
    pub first: Duration,
    /// The floor under the wait between one lost connection and the next attempt.
    pub retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            beat: BEAT,
            first: FIRST,
            retry: fetch::RETRY_NOW,
        }
    }
}

/// What the scheduler is lent: the ways to fetch, to listen, and to say what changed.
pub trait Host {
    /// Now. A window follows its own clock so a test can move it.
    fn now(&self) -> DateTime<Utc>;

    /// Who schedules now: this process, or a `mailo watch` beside it. Asked every time.
    fn schedule(&self) -> Schedule;

    /// Whether the background daemon holds the lock that makes it the one account watcher.
    fn daemon(&self) -> Daemon;

    /// Every account that can be pushed to, as the store describes them now.
    fn pushers(&self) -> Vec<AccountId>;

    /// Whether a folder of `account` is being fetched on demand, which a pass waits out because
    /// the two write the same rows.
    fn folder_in_flight(&self, account: &AccountId) -> bool;

    fn timing(&self) -> Timing {
        Timing::default()
    }

    /// Run one pass over `call.account`, saying where it is on `call.progress`.
    fn pass(&self, call: PassCall) -> Local<'_, Passed>;

    /// Hold one connection to `call.account` and say what it hears on `call.heard`, until
    /// `call.stop` or the connection is lost.
    fn listen(&self, call: ListenCall) -> Local<'_, Result<(), Lost>>;

    /// A change to report.
    fn changed(&self, change: Change);
}

/// When [`Scheduler::run`] returns of its own accord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Never: the owner drops it.
    Never,
    /// Once the accounts are known and every one has stopped for a reason only a person can
    /// clear: a credential the server refused, which no amount of retrying fixes.
    WhenStuck,
}

/// What the scheduler keeps for each pass, wait and live watch it has started.
///
/// Dropped with the scheduler: dropping a watch stops it, and a dropped cancel sender is read by
/// a pass as a cancellation.
struct Held {
    cancels: BTreeMap<AccountId, Arc<watch::Sender<bool>>>,
    passes: Generations,
    wakes: Generations,
    lives: Lives,
}

/// Whether the set of accounts has been heard yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Known {
    Not,
    Yes,
}

/// The effect runner: the one loop that moves every account's [`Link`].
pub struct Scheduler<'a> {
    host: &'a dyn Host,
    store: Arc<SqliteStore>,
    links: BTreeMap<AccountId, Link>,
    every: BTreeMap<AccountId, Duration>,
    tx: UnboundedSender<Note>,
    held: Held,
    fresh: Vec<Task<'a>>,
    known: Known,
}

/// How long to sleep for a wake at `at`, from `now`.
pub fn delay(at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (at - now)
        .max(TimeDelta::zero())
        .to_std()
        .unwrap_or(FAR)
        .min(FAR)
        + SLACK
}

impl std::fmt::Debug for Scheduler<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("links", &self.links)
            .finish_non_exhaustive()
    }
}

impl<'a> Scheduler<'a> {
    /// A scheduler over `links`, which are the accounts it already knows (see [`initial`]) with
    /// how often each wants a pass in `every`. `tx` is the sending end of the channel `run`
    /// will receive from, which the scheduler's own timers and watches send to as well.
    pub fn new(
        host: &'a dyn Host,
        store: Arc<SqliteStore>,
        links: BTreeMap<AccountId, Link>,
        every: BTreeMap<AccountId, Duration>,
        tx: UnboundedSender<Note>,
    ) -> Self {
        Scheduler {
            host,
            store,
            links,
            every,
            tx,
            held: Held {
                cancels: BTreeMap::new(),
                passes: Generations::default(),
                wakes: Generations::default(),
                lives: Lives::default(),
            },
            fresh: Vec::new(),
            known: Known::Not,
        }
    }

    /// Serve `inbox` until it closes, or until `end` says so. The first fetch of every account
    /// already known waits one beat. Returns each account's link as it stood.
    pub async fn run(
        mut self,
        mut inbox: UnboundedReceiver<Note>,
        end: End,
    ) -> Vec<(AccountId, Link)> {
        let known: Vec<AccountId> = self.links.keys().cloned().collect();
        if !known.is_empty() {
            let (tx, beat) = (self.tx.clone(), self.host.timing().beat);
            self.fresh.push(Box::pin(async move {
                tokio::time::sleep(beat).await;
                for account in known {
                    let _ = tx.send(Note::Event(account, Event::Start(Trigger::Poll)));
                }
            }));
        }
        let mut tasks = Tasks::default();
        loop {
            tasks.absorb(&mut self.fresh);
            if end == End::WhenStuck && self.known == Known::Yes && self.links.values().all(stuck) {
                break;
            }
            let heard = tokio::select! {
                note = inbox.recv() => Some(note),
                () = tasks.finished() => None,
            };
            match heard {
                Some(Some(note)) => self.note(note),
                Some(None) => break,
                None => {}
            }
        }
        self.links.into_iter().collect()
    }

    fn note(&mut self, note: Note) {
        match note {
            Note::Event(account, event) => self.handle(account, event),
            Note::Accounts(wanted) => self.follow(wanted),
        }
    }

    fn every(&self, account: &AccountId) -> Duration {
        self.every.get(account).copied().unwrap_or(FALLBACK)
    }

    /// Move one account's link by `event` and do what that asks.
    fn handle(&mut self, account: AccountId, event: Event) {
        let Some(link) = self.links.get(&account).cloned() else {
            return;
        };
        // Where a `mailo watch` runs, the timers and the server's pushes are its to answer.
        match verdict(self.host.schedule(), &link, &event) {
            Verdict::Run => {}
            Verdict::Defer => {
                self.defer(account.clone());
                self.reconcile_watch(&account, &link);
                return;
            }
            Verdict::Drop => {
                self.reconcile_watch(&account, &link);
                return;
            }
        }
        // What the live watch says about being connected is remembered whatever the link is
        // doing, because a link that is not current has nothing to apply it to yet.
        if let Event::Live(live) = &event
            && self.held.lives.watching(&account)
        {
            self.held.lives.remember(account.clone(), *live);
        }
        self.held.lives.dirty.heard(account.clone(), &link, &event);
        let now = self.host.now();
        let (next, effects) = fetch::step(&link, event.clone(), now, self.every(&account));
        if next != link {
            self.links.insert(account.clone(), next.clone());
            self.host
                .changed(Change::Link(account.clone(), next.clone()));
        }
        for effect in effects {
            self.effect(account.clone(), effect, now);
        }
        self.after(account, &link, &next, &event);
    }

    /// A poll or wake that was left to a watch is asked again in an interval, when the watch may
    /// be gone. One ask at a time: a later one replaces it.
    fn defer(&mut self, account: AccountId) {
        let generation = self.held.wakes.next(&account);
        let wakes = self.held.wakes.clone();
        let tx = self.tx.clone();
        let wait = self.every(&account);
        self.fresh.push(Box::pin(async move {
            tokio::time::sleep(wait).await;
            if wakes.current(&account, generation) {
                let _ = tx.send(Note::Event(account, Event::Start(Trigger::Poll)));
            }
        }));
    }

    /// What a step sets going besides its effects: the live watch, and a start that was waiting.
    fn after(&mut self, account: AccountId, was: &Link, next: &Link, event: &Event) {
        if was.is_busy() && !next.is_busy() {
            self.held.lives.passed(&account);
        }
        if std::mem::discriminant(was.resting()) != std::mem::discriminant(next.resting()) {
            self.reconcile_watch(&account, next);
        }
        // A start that found the pass running is made now that it has finished.
        if self
            .held
            .lives
            .dirty
            .settled(account.clone(), was, next, event)
            == Rerun::Yes
        {
            self.handle(account, Event::Start(Trigger::Push));
            return;
        }
        // A link that has just become current takes the watch's word for how it is kept up to
        // date, which a link that was fresh, waiting or syncing could not hold.
        if let Link::Current { live, .. } = next {
            let said = self.held.lives.said(&account);
            if *live != said {
                self.handle(account, Event::Live(said));
            }
        }
    }

    /// Start or stop `account`'s live watch to match `link`.
    fn reconcile_watch(&mut self, account: &AccountId, link: &Link) {
        let daemon = self.host.daemon();
        match self.held.lives.reconcile(account, link, daemon) {
            Action::Start => {
                let (stop, passes) = self.held.lives.begin(account);
                let timing = self.host.timing();
                self.fresh.push(Box::pin(keep(Keeping {
                    host: self.host,
                    account: account.clone(),
                    tx: self.tx.clone(),
                    stop,
                    passes,
                    first: timing.first,
                    retry: timing.retry,
                })));
            }
            // Its last word no longer holds: back on the timer.
            Action::Stop => self.handle(account.clone(), Event::Live(Live::Polling)),
            Action::Keep => {}
        }
    }

    fn effect(&mut self, account: AccountId, effect: Effect, now: DateTime<Utc>) {
        match effect {
            Effect::RunPass => {
                let (cancel, _) = watch::channel(false);
                let cancel = Arc::new(cancel);
                self.held.cancels.insert(account.clone(), cancel.clone());
                self.fresh.push(Box::pin(run_pass(Running {
                    host: self.host,
                    generation: self.held.passes.next(&account),
                    generations: self.held.passes.clone(),
                    tx: self.tx.clone(),
                    cancel,
                    account,
                })));
            }
            Effect::CancelPass => {
                // What the pass says from here on is about a pass that no longer counts.
                self.held.passes.next(&account);
                if let Some(cancel) = self.held.cancels.remove(&account) {
                    let _ = cancel.send(true);
                }
            }
            Effect::WakeAt(at) => {
                let generation = self.held.wakes.next(&account);
                let wakes = self.held.wakes.clone();
                let tx = self.tx.clone();
                let wait = delay(at, now);
                self.fresh.push(Box::pin(async move {
                    tokio::time::sleep(wait).await;
                    if wakes.current(&account, generation) {
                        let _ = tx.send(Note::Event(account, Event::Tick));
                    }
                }));
            }
        }
    }

    /// The accounts changed: give new ones a link, drop gone ones with their pass, and keep each
    /// one's interval. Then see which can be pushed to, which a pass may have just learned.
    fn follow(&mut self, wanted: Vec<(AccountId, Duration)>) {
        self.known = Known::Yes;
        self.every = wanted.iter().cloned().collect();
        let known = self.links.keys().cloned().collect();
        let changes = reconcile(&known, &wanted);
        let now = self.host.now();
        for account in changes.removed {
            if let Some(cancel) = self.held.cancels.remove(&account) {
                let _ = cancel.send(true);
            }
            self.held.passes.forget(&account);
            self.held.wakes.forget(&account);
            self.held.lives.forget(&account);
            self.links.remove(&account);
            self.host.changed(Change::Gone(account));
        }
        self.held.lives.pushers = self.host.pushers().into_iter().collect();
        for account in changes.added {
            let link = probe(&self.store, account.clone(), now);
            self.links.insert(account.clone(), link.clone());
            self.host.changed(Change::Link(account.clone(), link));
            self.handle(account, Event::Start(Trigger::Poll));
        }
        let all: Vec<(AccountId, Link)> = self
            .links
            .iter()
            .map(|(id, link)| (id.clone(), link.clone()))
            .collect();
        for (account, link) in all {
            self.reconcile_watch(&account, &link);
        }
    }
}

/// What a pass needs that the scheduler owns.
struct Running<'a> {
    host: &'a dyn Host,
    account: AccountId,
    generation: u64,
    generations: Generations,
    tx: UnboundedSender<Note>,
    /// Held for the whole pass: the engines read a dropped sender as a cancellation.
    cancel: Arc<watch::Sender<bool>>,
}

/// Run the pass, sending its progress and then its end to the scheduler.
///
/// Waits for a folder of the same account that is being fetched, because the two write the same
/// rows.
async fn run_pass(running: Running<'_>) {
    let Running {
        host,
        account,
        generation,
        generations,
        tx,
        cancel,
    } = running;
    while host.folder_in_flight(&account) {
        tokio::time::sleep(POLITE).await;
    }
    let (progress, mut said) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let call = PassCall {
        account: account.clone(),
        started: host.now(),
        cancel: cancel.subscribe(),
        progress,
    };
    let running = host.pass(call);
    let forward = async {
        // Ends when the pass drops its sender, which is when it has said everything.
        while let Some(event) = said.recv().await {
            if generations.current(&account, generation) {
                let _ = tx.send(Note::Event(account.clone(), event));
            }
        }
    };
    let (passed, ()) = tokio::join!(running, forward);
    if generations.current(&account, generation) {
        let _ = tx.send(Note::Event(account, passed.event));
    }
    // What a pass stored is what the list reads, cancelled or not.
    if passed.stored == Stored::Maybe {
        host.changed(Change::Stored);
    }
    drop(cancel);
}

#[cfg(test)]
mod tests;
