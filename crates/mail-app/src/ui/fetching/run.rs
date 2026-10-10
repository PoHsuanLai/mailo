//! The effect runner: the one loop that moves every account's [`Link`].
//!
//! Everything that happens to a link arrives here as a `(account, event)` pair: a person's
//! press, a pass's progress and end, a timer's tick. Each is given to [`fetch::step`], the next
//! link is stored for the window to read, and the effects it asked for are run. The link is
//! the state and this is the only thing that moves it, so there is no second place where
//! "is it syncing" is decided.

use super::delegate::{Verdict, verdict};
use super::live::{Action, Listener, Lives, Rerun, Wiring};
use super::pass::{Generations, Running, run as run_pass};
use super::{Note, start};
use chrono::{DateTime, TimeDelta, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::{Operation, PendingToken};
use mail_core::SqliteStore;
use mail_core::fetch::{self, Effect, Event, FolderFetch, Link, Live, Trigger};
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedSender, watch};

/// How soon a freshly opened window starts fetching, so that opening it is not also a network
/// round trip competing with the first paint.
pub(super) const BEAT: Duration = Duration::from_secs(2);

/// The interval an account is polled at when nothing says otherwise.
const FALLBACK: Duration = Duration::from_secs(300);

/// A timer is set for a little after the time it is for, so that it never wakes a link a
/// moment before the link would agree it is due.
const SLACK: Duration = Duration::from_millis(250);

/// The longest a single timer sleeps. A wake that is further off than this is asked for again.
const FAR: Duration = Duration::from_secs(24 * 3600);

/// What the runner reads and writes.
pub(super) struct Runner {
    pub links: Signal<BTreeMap<AccountId, Link>>,
    pub ops: Signal<BTreeMap<AccountId, Operation>>,
    pub folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    pub revision: Signal<u64>,
    pub tx: UnboundedSender<Note>,
    pub store: Arc<SqliteStore>,
    pub passer: super::pass::Passer,
    pub delegate: super::delegate::Delegate,
    pub every: BTreeMap<AccountId, Duration>,
}

/// What the runner keeps for each pass, wait and live watch it has started.
///
/// Dropped with the runner, which is when the window closes: dropping a watch stops it, and a
/// dropped cancel sender is read by a pass as a cancellation.
struct Held {
    cancels: BTreeMap<AccountId, Arc<watch::Sender<bool>>>,
    passes: Generations,
    wakes: Generations,
    lives: Lives,
}

impl Held {
    fn new(listener: Listener) -> Self {
        Self {
            cancels: BTreeMap::new(),
            passes: Generations::default(),
            wakes: Generations::default(),
            lives: Lives::new(listener),
        }
    }
}

/// The operation to show once `link` is what it is: running only while a pass is.
pub(super) fn settled(op: Operation, link: &Link) -> Operation {
    if link.is_busy() { op } else { Operation::Idle }
}

/// How long to sleep for a wake at `at`, from `now`.
pub(super) fn delay(at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (at - now)
        .max(TimeDelta::zero())
        .to_std()
        .unwrap_or(FAR)
        .min(FAR)
        + SLACK
}

impl Runner {
    /// Serve the channel until the window closes it. The first fetch of every account waits one
    /// beat.
    pub(super) async fn run(mut self, mut rx: tokio::sync::mpsc::UnboundedReceiver<Note>) {
        let mut held =
            Held::new(try_consume_context::<Listener>().unwrap_or_else(Listener::server));
        let known: Vec<AccountId> = self.links.peek().keys().cloned().collect();
        let tx = self.tx.clone();
        spawn(async move {
            tokio::time::sleep(BEAT).await;
            for account in known {
                let _ = tx.send(Note::Event(account, Event::Start(Trigger::Poll)));
            }
        });
        while let Some(note) = rx.recv().await {
            match note {
                Note::Event(account, event) => self.handle(&mut held, account, event),
                Note::Accounts(wanted) => self.follow(&mut held, wanted),
            }
        }
    }

    fn every(&self, account: AccountId) -> Duration {
        self.every.get(&account).copied().unwrap_or(FALLBACK)
    }

    /// Move one account's link by `event` and do what that asks.
    fn handle(&mut self, held: &mut Held, account: AccountId, event: Event) {
        let Some(link) = self.links.peek().get(&account).cloned() else {
            return;
        };
        // Where a `mailo watch` runs, the timers and the server's pushes are its to answer.
        match verdict(self.delegate.schedule(), &link, &event) {
            Verdict::Run => {}
            Verdict::Defer => {
                self.defer(held, account.clone());
                self.watch(held, account, &link);
                return;
            }
            Verdict::Drop => {
                self.watch(held, account, &link);
                return;
            }
        }
        // What the live watch says about being connected is remembered whatever the link is
        // doing, because a link that is not current has nothing to apply it to yet.
        if let Event::Live(live) = &event
            && held.lives.watching(account.clone())
        {
            held.lives.remember(account.clone(), *live);
        }
        held.lives.dirty.heard(account.clone(), &link, &event);
        let now = crate::ui::clock::now();
        let (next, effects) = fetch::step(&link, event.clone(), now, self.every(account.clone()));
        let before = self.ops.peek().get(&account).copied().unwrap_or_default();
        let op = settled(before, &next);
        if next != link {
            self.links.write().insert(account.clone(), next.clone());
        }
        if op != before {
            self.ops.write().insert(account.clone(), op);
        }
        for effect in effects {
            self.effect(held, account.clone(), effect, now);
        }
        self.after(held, account, &link, &next, &event);
    }

    /// A poll or wake that was left to a watch is asked again in an interval, when the watch may
    /// be gone. One ask at a time: a later one replaces it.
    fn defer(&mut self, held: &mut Held, account: AccountId) {
        let generation = held.wakes.next(account.clone());
        let wakes = held.wakes.clone();
        let tx = self.tx.clone();
        let wait = self.every(account.clone());
        spawn(async move {
            tokio::time::sleep(wait).await;
            if wakes.current(account.clone(), generation) {
                let _ = tx.send(Note::Event(account, Event::Start(Trigger::Poll)));
            }
        });
    }

    /// What a step sets going besides its effects: the live watch, and a start that was waiting.
    fn after(
        &mut self,
        held: &mut Held,
        account: AccountId,
        was: &Link,
        next: &Link,
        event: &Event,
    ) {
        if was.is_busy() && !next.is_busy() {
            held.lives.passed(account.clone());
        }
        if std::mem::discriminant(was.resting()) != std::mem::discriminant(next.resting()) {
            self.watch(held, account.clone(), next);
        }
        // A start that found the pass running is made now that it has finished.
        if held.lives.dirty.settled(account.clone(), was, next, event) == Rerun::Yes {
            self.handle(held, account, Event::Start(Trigger::Push));
            return;
        }
        // A link that has just become current takes the watch's word for how it is kept up to
        // date, which a link that was fresh, waiting or syncing could not hold.
        if let Link::Current { live, .. } = next {
            let said = held.lives.said(account.clone());
            if *live != said {
                self.handle(held, account, Event::Live(said));
            }
        }
    }

    /// Start or stop `account`'s live watch to match `link`.
    fn watch(&mut self, held: &mut Held, account: AccountId, link: &Link) {
        let wiring = Wiring {
            store: &self.store,
            tx: &self.tx,
        };
        if held.lives.reconcile(account.clone(), link, &wiring) == Action::Stop {
            // Its last word no longer holds: back on the timer.
            self.handle(held, account, Event::Live(Live::Polling));
        }
    }

    fn effect(&mut self, held: &mut Held, account: AccountId, effect: Effect, now: DateTime<Utc>) {
        match effect {
            Effect::RunPass => {
                self.ops
                    .write()
                    .insert(account.clone(), Operation::Running(PendingToken::start()));
                let (cancel, _) = watch::channel(false);
                let cancel = Arc::new(cancel);
                held.cancels.insert(account.clone(), cancel.clone());
                spawn(run_pass(Running {
                    account: account.clone(),
                    generation: held.passes.next(account),
                    generations: held.passes.clone(),
                    tx: self.tx.clone(),
                    store: self.store.clone(),
                    passer: self.passer.clone(),
                    cancel,
                    folders: self.folders,
                    revision: self.revision,
                }));
            }
            Effect::CancelPass => {
                // What the pass says from here on is about a pass that no longer counts.
                held.passes.next(account.clone());
                if let Some(cancel) = held.cancels.remove(&account) {
                    let _ = cancel.send(true);
                }
            }
            Effect::WakeAt(at) => {
                let generation = held.wakes.next(account.clone());
                let wakes = held.wakes.clone();
                let tx = self.tx.clone();
                let wait = delay(at, now);
                spawn(async move {
                    tokio::time::sleep(wait).await;
                    if wakes.current(account.clone(), generation) {
                        let _ = tx.send(Note::Event(account, Event::Tick));
                    }
                });
            }
        }
    }

    /// The accounts changed: give new ones a link, drop gone ones with their pass, and keep each
    /// one's interval. Then see which can be pushed to, which a pass may have just learned.
    fn follow(&mut self, held: &mut Held, wanted: Vec<(AccountId, Duration)>) {
        self.every = wanted.iter().cloned().collect();
        let known = self.links.peek().keys().cloned().collect();
        let changes = start::reconcile(&known, &wanted);
        let now = crate::ui::clock::now();
        for account in changes.removed {
            if let Some(cancel) = held.cancels.remove(&account) {
                let _ = cancel.send(true);
            }
            held.passes.forget(account.clone());
            held.wakes.forget(account.clone());
            held.lives.forget(account.clone());
            self.links.write().remove(&account);
            self.ops.write().remove(&account);
            self.folders.write().retain(|(one, _), _| *one != account);
        }
        held.lives.refresh(&self.store);
        for account in changes.added {
            self.links.write().insert(
                account.clone(),
                start::probe(&self.store, account.clone(), now),
            );
            self.ops.write().insert(account.clone(), Operation::Idle);
            self.handle(held, account, Event::Start(Trigger::Poll));
        }
        let all: Vec<(AccountId, Link)> = self
            .links
            .peek()
            .iter()
            .map(|(id, link)| (id.clone(), link.clone()))
            .collect();
        for (account, link) in all {
            self.watch(held, account, &link);
        }
    }
}

#[cfg(test)]
mod tests;
