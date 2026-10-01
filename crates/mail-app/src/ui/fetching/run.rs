//! The effect runner: the one loop that moves every account's [`Link`].
//!
//! Everything that happens to a link arrives here as a `(account, event)` pair: a person's
//! press, a pass's progress and end, a timer's tick. Each is given to [`fetch::step`], the next
//! link is stored for the window to read, and the effects it asked for are run. The link is
//! the state and this is the only thing that moves it, so there is no second place where
//! "is it syncing" is decided.

use super::pass::{Generations, Running, run as run_pass};
use super::{Note, start};
use crate::fetch::{self, Effect, Event, FolderFetch, Link, Trigger};
use chrono::{DateTime, TimeDelta, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::{Operation, PendingToken};
use mail_domain::AccountId;
use mail_store::SqliteStore;
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
    pub every: BTreeMap<AccountId, Duration>,
}

/// What the runner keeps for each pass and wait it has started.
#[derive(Default)]
struct Held {
    cancels: BTreeMap<AccountId, Arc<watch::Sender<bool>>>,
    passes: Generations,
    wakes: Generations,
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
        let mut held = Held::default();
        let known: Vec<AccountId> = self.links.peek().keys().copied().collect();
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
        let now = crate::ui::clock::now();
        let (next, effects) = fetch::step(&link, event, now, self.every(account));
        let before = self.ops.peek().get(&account).copied().unwrap_or_default();
        let op = settled(before, &next);
        if next != link {
            self.links.write().insert(account, next);
        }
        if op != before {
            self.ops.write().insert(account, op);
        }
        for effect in effects {
            self.effect(held, account, effect, now);
        }
    }

    fn effect(&mut self, held: &mut Held, account: AccountId, effect: Effect, now: DateTime<Utc>) {
        match effect {
            Effect::RunPass => {
                self.ops
                    .write()
                    .insert(account, Operation::Running(PendingToken::start()));
                let (cancel, _) = watch::channel(false);
                let cancel = Arc::new(cancel);
                held.cancels.insert(account, cancel.clone());
                spawn(run_pass(Running {
                    account,
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
                held.passes.next(account);
                if let Some(cancel) = held.cancels.remove(&account) {
                    let _ = cancel.send(true);
                }
            }
            Effect::WakeAt(at) => {
                let generation = held.wakes.next(account);
                let wakes = held.wakes.clone();
                let tx = self.tx.clone();
                let wait = delay(at, now);
                spawn(async move {
                    tokio::time::sleep(wait).await;
                    if wakes.current(account, generation) {
                        let _ = tx.send(Note::Event(account, Event::Tick));
                    }
                });
            }
        }
    }

    /// The accounts changed: give new ones a link, drop gone ones with their pass, and keep each
    /// one's interval.
    fn follow(&mut self, held: &mut Held, wanted: Vec<(AccountId, Duration)>) {
        self.every = wanted.iter().copied().collect();
        let known = self.links.peek().keys().copied().collect();
        let changes = start::reconcile(&known, &wanted);
        let now = crate::ui::clock::now();
        for account in changes.removed {
            if let Some(cancel) = held.cancels.remove(&account) {
                let _ = cancel.send(true);
            }
            held.passes.forget(account);
            held.wakes.forget(account);
            self.links.write().remove(&account);
            self.ops.write().remove(&account);
            self.folders.write().retain(|(one, _), _| *one != account);
        }
        for account in changes.added {
            self.links
                .write()
                .insert(account, start::probe(&self.store, account, now));
            self.ops.write().insert(account, Operation::Idle);
            self.handle(held, account, Event::Start(Trigger::Poll));
        }
    }
}

#[cfg(test)]
mod tests;
