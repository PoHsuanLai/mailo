//! Push in the window: one long-lived watch per account that the server can push to.
//!
//! [`mail_core::sync::live::listen`] holds the connection (IMAP `IDLE`, JMAP's event source) and
//! says what it hears; [`keep`] turns that into the events the link reads and reconnects when
//! the connection is lost. The watch runs no pass and takes no part in one: a wake for mail is
//! a `Start(Trigger::Push)`, and the link decides whether a pass can begin.
//!
//! Three things keep that honest:
//!
//! - **A wake that finds a pass running is not lost.** The link ignores a start while it is
//!   syncing, so [`Dirty`] remembers it and the runner starts again when that pass finishes.
//! - **The watch does not hear its own pass as news.** Until a pass has stored what the wake was
//!   about, waiting again would be told about it again; [`mail_core::sync::live::Hold`] is how
//!   the runner says a pass has ended.
//! - **Not both a daemon and a window.** When the background daemon holds the lock, or a
//!   `mailo watch` runs ([`Daemon::Holding`]), the window does not watch: two connections to one
//!   server for one account is what the lock exists to prevent. With a `mailo watch` the window
//!   leaves scheduled fetching to it altogether (`super::delegate`); with only the daemon, which
//!   syncs on request and holds no `IDLE` of its own, it polls on its timer as an account
//!   without push does.

use super::Note;
use mail_core::fetch::{self, BACKOFF_CEILING, Event, Link, Live, RETRY_NOW, Trigger};
use mail_core::sync::live::{self as core, Heard, Lost};
use mail_domain::Retry;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedSender, watch};

/// How a started watch is told it may stop: the sender is dropped or set.
type Stop = watch::Receiver<bool>;

/// Runs one connection of a watch: the signature of [`core::listen`], with the grace chosen.
type Listen = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, Stop, core::Hold, &dyn Fn(Heard)) -> Result<(), Lost>
        + Send
        + Sync,
>;

/// Which accounts the server can push to: the signature of [`core::pushing`].
type Pushers = Arc<dyn Fn(&SqliteStore) -> Vec<AccountId> + Send + Sync>;

/// Whether the background daemon holds the lock that makes it the one account watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Daemon {
    Holding,
    Absent,
}

/// Whether the server can push to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Push {
    Offered,
    Absent,
}

/// What the watch is, and what it asks of its surroundings. The real ones unless a test
/// provided its own, as [`super::Passer`] is.
#[derive(Clone)]
pub(in crate::ui) struct Listener {
    listen: Listen,
    /// Every account that can be pushed to, as the store describes them now.
    pushers: Pushers,
    daemon: Arc<dyn Fn() -> Daemon + Send + Sync>,
    /// How long after being wanted a watch connects, so that opening a window is not also a
    /// second connection racing the first pass.
    first: Duration,
    /// The floor under the wait between one lost connection and the next attempt.
    retry: Duration,
}

impl Listener {
    /// The servers, through [`core::listen`]; the daemon, through its lock.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self {
            listen: Arc::new(|store, account, stop, hold, heard| {
                core::listen(store, account, stop, Some(hold), core::GRACE, heard)
            }),
            pushers: Arc::new(core::pushing),
            daemon: Arc::new(|| {
                // Racy by nature, as the lock's own documentation says: a daemon that starts a
                // moment later is noticed the next time a link rests or the accounts change.
                match mail_core::ipc::agent() {
                    Ok(agent) if agent.is_running() => Daemon::Holding,
                    // A `mailo watch` is the one that holds the connection, and the window
                    // leaves scheduled fetching to it (`super::delegate`).
                    _ if mail_core::ipc::watching::running() => Daemon::Holding,
                    _ => Daemon::Absent,
                }
            }),
            first: Duration::from_secs(4),
            retry: RETRY_NOW,
        }
    }

    /// Not in a test build: a watch opens sockets, and a test that forgot to provide its own
    /// must not reach one. No account is pushed to, so none is watched.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self {
            listen: Arc::new(|_, _, _, _, _| {
                Err(Lost {
                    retry: Retry::Fatal("a test watched without providing a Listener".to_owned()),
                    why: "a test watched without providing a Listener".to_owned(),
                })
            }),
            pushers: Arc::new(|_| Vec::new()),
            daemon: Arc::new(|| Daemon::Absent),
            first: Duration::ZERO,
            retry: RETRY_NOW,
        }
    }
}

/// What a watch hears, as what the link is told.
pub(super) fn event_for(heard: Heard) -> Event {
    match heard {
        Heard::Established => Event::Live(Live::Pushed),
        Heard::Mail => Event::Start(Trigger::Push),
        // A send came due, or the wait ended without news. Neither is the server speaking, so
        // neither is a push; both are a reason to look, which is the only polling a pushed
        // account does.
        Heard::Due | Heard::Interval => Event::Start(Trigger::Poll),
    }
}

/// What a watch does about a lost connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reconnect {
    /// How long before the next attempt.
    pub wait: Duration,
    /// What to tell the link meanwhile, if the loss is something a pass should find out.
    pub nudge: Option<Event>,
}

/// How long to wait after the `failures`th loss in a row, which was `retry`, with `floor` as the
/// shortest wait.
///
/// The same doubling as a failing pass ([`fetch::backoff`]), never spinning. A refused sign-in
/// is not retried in a loop (F128): a pass is asked to find that out, which sets the link to
/// NeedsSignIn and stops the watch, and the watch only looks again at the ceiling.
pub(super) fn reconnect(failures: u32, retry: &Retry, floor: Duration) -> Reconnect {
    let schedule = fetch::backoff(failures, floor);
    match retry {
        Retry::NeedsReauth => Reconnect {
            wait: BACKOFF_CEILING,
            nudge: Some(Event::Start(Trigger::Poll)),
        },
        Retry::Fatal(_) => Reconnect {
            wait: BACKOFF_CEILING,
            nudge: None,
        },
        Retry::After(asked) => Reconnect {
            wait: schedule.max(*asked),
            nudge: None,
        },
        Retry::Now => Reconnect {
            wait: schedule,
            nudge: None,
        },
    }
}

/// What to do about an account's watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Start,
    Stop,
    Keep,
}

/// Whether a watch is running, given what is true now.
///
/// Wanted when the server can push, nobody else holds the account, and the link is not at a stop
/// that only a person can clear: a refused credential or a broken account. A link that is
/// waiting out a failure still has its watch, which is how the first mail after the server comes
/// back is heard.
pub(super) fn decide(running: bool, push: Push, daemon: Daemon, link: &Link) -> Action {
    let wanted = push == Push::Offered
        && daemon == Daemon::Absent
        && !matches!(
            link.resting(),
            Link::NeedsSignIn { .. } | Link::Broken { .. }
        );
    match (running, wanted) {
        (false, true) => Action::Start,
        (true, false) => Action::Stop,
        _ => Action::Keep,
    }
}

/// Whether a start that was ignored should be tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rerun {
    Yes,
    No,
}

/// The accounts that were told of mail while a pass was running.
#[derive(Default)]
pub(super) struct Dirty(BTreeSet<AccountId>);

impl Dirty {
    /// `event` is about to reach `link`: remember it if the link will ignore it for being busy.
    pub fn heard(&mut self, account: AccountId, link: &Link, event: &Event) {
        if link.is_busy() && matches!(event, Event::Start(Trigger::Push)) {
            self.0.insert(account);
        }
    }

    /// `event` moved `before` to `after`: whether a start remembered earlier is due now.
    ///
    /// Only a pass that finished and left the account current. A cancelled one is not to be
    /// restarted behind a person's back, and a failed one has its own backoff.
    pub fn settled(
        &mut self,
        account: AccountId,
        before: &Link,
        after: &Link,
        event: &Event,
    ) -> Rerun {
        if !before.is_busy() || after.is_busy() {
            return Rerun::No;
        }
        let owed = self.0.remove(&account);
        match (owed, event, after) {
            (true, Event::Finished { .. }, Link::Current { .. }) => Rerun::Yes,
            _ => Rerun::No,
        }
    }

    pub fn forget(&mut self, account: AccountId) {
        self.0.remove(&account);
    }
}

/// What a running watch is, to the runner. Dropping it stops the watch.
struct Watch {
    stop: watch::Sender<bool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// Every account's watch, and what the runner remembers about them.
pub(super) struct Lives {
    pub listener: Listener,
    pub dirty: Dirty,
    /// The accounts the server can push to, as of the last time the accounts changed.
    pub pushers: BTreeSet<AccountId>,
    running: BTreeMap<AccountId, Watch>,
    /// What each watch last said about being connected.
    said: BTreeMap<AccountId, Live>,
    /// Bumped each time one of the account's passes ends: [`core::Hold`].
    passes: BTreeMap<AccountId, watch::Sender<u64>>,
}

/// What starting a watch needs that the runner owns.
pub(super) struct Wiring<'a> {
    pub store: &'a Arc<SqliteStore>,
    pub tx: &'a UnboundedSender<Note>,
}

impl Lives {
    pub fn new(listener: Listener) -> Self {
        Self {
            listener,
            dirty: Dirty::default(),
            pushers: BTreeSet::new(),
            running: BTreeMap::new(),
            said: BTreeMap::new(),
            passes: BTreeMap::new(),
        }
    }

    /// Re-read which accounts the server can push to.
    pub fn refresh(&mut self, store: &SqliteStore) {
        self.pushers = (self.listener.pushers)(store).into_iter().collect();
    }

    /// What the watch of `account` last said about being connected.
    pub fn said(&self, account: AccountId) -> Live {
        self.said.get(&account).copied().unwrap_or(Live::Polling)
    }

    /// Whether `account` has a watch running.
    pub fn watching(&self, account: AccountId) -> bool {
        self.running.contains_key(&account)
    }

    pub fn remember(&mut self, account: AccountId, live: Live) {
        self.said.insert(account, live);
    }

    /// A pass of `account` has ended.
    pub fn passed(&mut self, account: AccountId) {
        if let Some(sender) = self.passes.get(&account) {
            sender.send_modify(|n| *n += 1);
        }
    }

    /// Start or stop `account`'s watch to match `link`. Returns whether a running watch was
    /// stopped, which is when its last word on being connected no longer holds.
    pub fn reconcile(&mut self, account: AccountId, link: &Link, wiring: &Wiring<'_>) -> Action {
        let push = if self.pushers.contains(&account) {
            Push::Offered
        } else {
            Push::Absent
        };
        let daemon = (self.listener.daemon)();
        let action = decide(self.running.contains_key(&account), push, daemon, link);
        match action {
            Action::Start => self.start(account, wiring),
            Action::Stop => self.stop(account),
            Action::Keep => {}
        }
        action
    }

    fn start(&mut self, account: AccountId, wiring: &Wiring<'_>) {
        let (stop, stopped) = watch::channel(false);
        let passes = self
            .passes
            .entry(account.clone())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe();
        self.running.insert(account.clone(), Watch { stop });
        dioxus::prelude::spawn(keep(Keeping {
            account,
            store: wiring.store.clone(),
            tx: wiring.tx.clone(),
            stop: stopped,
            passes,
            listener: self.listener.clone(),
        }));
    }

    /// Stop the watch, if any, and forget what it said.
    pub fn stop(&mut self, account: AccountId) {
        self.running.remove(&account);
        self.said.remove(&account);
    }

    /// The account is gone.
    pub fn forget(&mut self, account: AccountId) {
        self.stop(account.clone());
        self.passes.remove(&account);
        self.dirty.forget(account);
    }
}

/// What a watch's task needs.
struct Keeping {
    account: AccountId,
    store: Arc<SqliteStore>,
    tx: UnboundedSender<Note>,
    stop: Stop,
    passes: watch::Receiver<u64>,
    listener: Listener,
}

/// Whether the watch has been told to stop, or its owner is gone.
fn stopped(stop: &Stop) -> bool {
    stop.has_changed().map_or(true, |_| *stop.borrow())
}

/// Sleep for `wait`, or until stopped. `false` when stopped.
async fn rest(wait: Duration, stop: &mut Stop) -> bool {
    tokio::select! {
        () = tokio::time::sleep(wait) => !stopped(stop),
        _ = stop.changed() => !stopped(stop),
    }
}

/// Keep one account's watch alive: connect, say what is heard, and when the connection is lost
/// say so and connect again, further apart each time it fails. Ends when stopped.
async fn keep(mut k: Keeping) {
    let account = k.account;
    let send = |event: Event| {
        let _ = k.tx.send(Note::Event(account.clone(), event));
    };
    if !rest(k.listener.first, &mut k.stop).await {
        return;
    }
    let mut failures = 0u32;
    while !stopped(&k.stop) {
        let up = Arc::new(AtomicBool::new(false));
        let ended = {
            let (store, stop, hold) = (k.store.clone(), k.stop.clone(), k.passes.clone());
            let (tx, listen, up) = (k.tx.clone(), k.listener.listen.clone(), up.clone());
            let account = account.clone();
            // A thread of its own: the watch builds a runtime, and parks in a socket.
            tokio::task::spawn_blocking(move || {
                let heard = |heard: Heard| {
                    if heard == Heard::Established {
                        up.store(true, Ordering::SeqCst);
                    }
                    let _ = tx.send(Note::Event(account.clone(), event_for(heard)));
                };
                listen(store, account.clone(), stop, hold, &heard)
            })
            .await
        };
        let lost = match ended {
            Ok(Ok(())) => return,
            Ok(Err(lost)) => lost,
            Err(e) => Lost {
                retry: Retry::Fatal(e.to_string()),
                why: format!("the watch stopped: {e}"),
            },
        };
        if stopped(&k.stop) {
            return;
        }
        send(Event::Live(Live::Polling));
        // A connection that was held and then lost starts the schedule again.
        failures = if up.load(Ordering::SeqCst) {
            1
        } else {
            failures.saturating_add(1)
        };
        let next = reconnect(failures, &lost.retry, k.listener.retry);
        if let Some(nudge) = next.nudge {
            send(nudge);
        }
        if !rest(next.wait, &mut k.stop).await {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
