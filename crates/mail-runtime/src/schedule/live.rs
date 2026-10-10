//! Push: one long-lived watch per account that the server can push to.
//!
//! The [`Host`] holds the connection (IMAP `IDLE`, JMAP's event source) and says what it hears;
//! [`keep`] turns that into the events the link reads and reconnects when the connection is
//! lost. The watch runs no pass and takes no part in one: a wake for mail is a
//! `Start(Trigger::Push)`, and the link decides whether a pass can begin.
//!
//! Three things keep that honest:
//!
//! - **A wake that finds a pass running is not lost.** The link ignores a start while it is
//!   syncing, so [`Dirty`] remembers it and the scheduler starts again when that pass finishes.
//! - **The watch does not hear its own pass as news.** Until a pass has stored what the wake was
//!   about, waiting again would be told about it again; [`Hold`] is how the scheduler says a
//!   pass has ended.
//! - **Not both a daemon and a window.** When the background daemon holds the lock, or a
//!   `mailo watch` runs ([`Daemon::Holding`]), the window does not watch: two connections to one
//!   server for one account is what the lock exists to prevent. With a `mailo watch` the window
//!   leaves scheduled fetching to it altogether ([`super::Schedule`]); with only the daemon,
//!   which syncs on request and holds no `IDLE` of its own, it polls on its timer as an account
//!   without push does.

use super::{Host, Note};
use crate::Woke;
use crate::fetch::{self, BACKOFF_CEILING, Event, Link, Live, Trigger};
use mail_domain::Retry;
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedSender, watch};

/// How a started watch is told it may stop: the sender is dropped or set.
pub type Stop = watch::Receiver<bool>;

/// How the scheduler lets a watch know it has finished reacting to a wake.
///
/// After [`Heard::Mail`] the stored cursor is out of date until the pass has run, so waiting
/// again at once would be told about the same mail again, in a loop. The scheduler bumps this
/// channel when a pass of the account ends; the watch waits for that, for a bounded time,
/// before it listens again.
pub type Hold = watch::Receiver<u64>;

/// What the server, the outbox or the clock said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// The connection is held and the server is being waited on.
    Established,
    /// The server said something changed.
    Mail,
    /// Something in the outbox came due.
    Due,
    /// The server ended its wait without news, or the interval ran out. Mail may have arrived
    /// in the gap between one wait and the next, so this is when a quiet account is looked at.
    Interval,
}

impl From<Woke> for Heard {
    /// What a wake is told as.
    fn from(woke: Woke) -> Self {
        match woke {
            Woke::Mail => Heard::Mail,
            Woke::Due => Heard::Due,
            Woke::Interval => Heard::Interval,
        }
    }
}

/// Why the watch is over, when nobody asked it to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lost {
    /// What to do about it: wait, ask for a sign-in, or give up.
    pub retry: Retry,
    /// What to tell a person.
    pub why: String,
}

/// What a watch hears, as what the link is told.
pub fn event_for(heard: Heard) -> Event {
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
pub struct Reconnect {
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
pub fn reconnect(failures: u32, retry: &Retry, floor: Duration) -> Reconnect {
    let schedule = fetch::backoff(failures, floor);
    match retry {
        Retry::NeedsReauth | Retry::NeedsGrant => Reconnect {
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

/// Whether the background daemon holds the lock that makes it the one account watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Daemon {
    Holding,
    Absent,
}

/// Whether the server can push to an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    Offered,
    Absent,
}

/// What to do about an account's watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
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
pub fn decide(running: bool, push: Push, daemon: Daemon, link: &Link) -> Action {
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
pub enum Rerun {
    Yes,
    No,
}

/// The accounts that were told of mail while a pass was running.
#[derive(Debug, Default)]
pub struct Dirty(BTreeSet<AccountId>);

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

    pub fn forget(&mut self, account: &AccountId) {
        self.0.remove(account);
    }
}

/// What a running watch is, to the scheduler. Dropping it stops the watch.
struct Watch {
    stop: watch::Sender<bool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// Every account's watch, and what the scheduler remembers about them.
#[derive(Default)]
pub(super) struct Lives {
    pub dirty: Dirty,
    /// The accounts the server can push to, as of the last time the accounts changed.
    pub pushers: BTreeSet<AccountId>,
    running: BTreeMap<AccountId, Watch>,
    /// What each watch last said about being connected.
    said: BTreeMap<AccountId, Live>,
    /// Bumped each time one of the account's passes ends: [`Hold`].
    passes: BTreeMap<AccountId, watch::Sender<u64>>,
}

impl Lives {
    /// What the watch of `account` last said about being connected.
    pub fn said(&self, account: &AccountId) -> Live {
        self.said.get(account).copied().unwrap_or(Live::Polling)
    }

    /// Whether `account` has a watch running.
    pub fn watching(&self, account: &AccountId) -> bool {
        self.running.contains_key(account)
    }

    pub fn remember(&mut self, account: AccountId, live: Live) {
        self.said.insert(account, live);
    }

    /// A pass of `account` has ended.
    pub fn passed(&mut self, account: &AccountId) {
        if let Some(sender) = self.passes.get(account) {
            sender.send_modify(|n| *n += 1);
        }
    }

    /// What `account`'s watch should do to match `link`. A watch that is no longer wanted is
    /// stopped here, which is when its last word on being connected no longer holds; one that
    /// is wanted is for the caller to [`begin`](Self::begin).
    pub fn reconcile(&mut self, account: &AccountId, link: &Link, daemon: Daemon) -> Action {
        let push = if self.pushers.contains(account) {
            Push::Offered
        } else {
            Push::Absent
        };
        let action = decide(self.running.contains_key(account), push, daemon, link);
        if action == Action::Stop {
            self.stop(account);
        }
        action
    }

    /// Note that a watch of `account` is running, and give it what it listens with.
    pub fn begin(&mut self, account: &AccountId) -> (Stop, Hold) {
        let (stop, stopped) = watch::channel(false);
        let passes = self
            .passes
            .entry(account.clone())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe();
        self.running.insert(account.clone(), Watch { stop });
        (stopped, passes)
    }

    /// Stop the watch, if any, and forget what it said.
    pub fn stop(&mut self, account: &AccountId) {
        self.running.remove(account);
        self.said.remove(account);
    }

    /// The account is gone.
    pub fn forget(&mut self, account: &AccountId) {
        self.stop(account);
        self.passes.remove(account);
        self.dirty.forget(account);
    }
}

/// What a watch's task needs.
pub(super) struct Keeping<'a> {
    pub host: &'a dyn Host,
    pub account: AccountId,
    pub tx: UnboundedSender<Note>,
    pub stop: Stop,
    pub passes: Hold,
    pub first: Duration,
    pub retry: Duration,
}

/// What the host is asked to hold one connection with.
pub struct ListenCall {
    pub account: AccountId,
    /// Ends the wait: set, or its sender dropped.
    pub stop: Stop,
    pub hold: Hold,
    /// Says what the connection hears. Callable from any thread.
    pub heard: Arc<dyn Fn(Heard) + Send + Sync>,
}

impl std::fmt::Debug for ListenCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenCall")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
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
pub(super) async fn keep(mut k: Keeping<'_>) {
    let account = k.account.clone();
    let send = |event: Event| {
        let _ = k.tx.send(Note::Event(account.clone(), event));
    };
    if !rest(k.first, &mut k.stop).await {
        return;
    }
    let mut failures = 0u32;
    while !stopped(&k.stop) {
        let up = Arc::new(AtomicBool::new(false));
        let heard: Arc<dyn Fn(Heard) + Send + Sync> = {
            let (tx, account, up) = (k.tx.clone(), account.clone(), up.clone());
            Arc::new(move |heard| {
                if heard == Heard::Established {
                    up.store(true, Ordering::SeqCst);
                }
                let _ = tx.send(Note::Event(account.clone(), event_for(heard)));
            })
        };
        let ended = k
            .host
            .listen(ListenCall {
                account: account.clone(),
                stop: k.stop.clone(),
                hold: k.passes.clone(),
                heard,
            })
            .await;
        let lost = match ended {
            Ok(()) => return,
            Err(lost) => lost,
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
        let next = reconnect(failures, &lost.retry, k.retry);
        if let Some(nudge) = next.nudge {
            send(nudge);
        }
        if !rest(next.wait, &mut k.stop).await {
            return;
        }
    }
}
