//! One account's connection to its server, as a state machine.
//!
//! [`step`] is the whole of it: a state, an event and the time go in, the next state and the
//! effects the caller must perform come out. Nothing here reads a clock or opens a socket, so the
//! backoff rule that used to live in `view::next_sync` is a table in the tests rather than
//! something to wait for.

use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::Retry;
use std::time::Duration;

/// The longest a failing account is left alone between attempts.
///
/// Half an hour. Long enough that a server which is down for the afternoon is not asked every
/// five minutes, short enough that mail is not an hour stale once it comes back.
pub const BACKOFF_CEILING: Duration = Duration::from_secs(30 * 60);

/// How long a dropped connection waits before the one retry that is "at once".
///
/// Not zero: a server that has just reset the connection is no likelier to accept the very next
/// packet, and a loop with no wait is a loop that hammers.
pub const RETRY_NOW: Duration = Duration::from_secs(5);

/// Where an account's mail fetching stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// No pass has completed yet. Nothing is known about the mailbox.
    Fresh,
    /// A pass is running.
    Syncing {
        /// Whether this is the pass that fills an empty account.
        first: First,
        /// What the pass is doing now.
        step: Step,
        /// How far through that step, when it knows.
        count: Option<Count>,
        /// The resting state to return to if the pass is cancelled.
        after: Box<Link>,
    },
    /// The last pass finished.
    Current {
        /// When it finished.
        at: DateTime<Utc>,
        /// Folders it could not update. Empty when everything went through.
        trouble: Vec<Trouble>,
        /// Whether the server tells us about new mail or we have to ask.
        live: Live,
    },
    /// The last pass failed in a way that waiting may fix.
    Waiting {
        /// When to try again.
        until: DateTime<Utc>,
        /// What the server did.
        why: Pause,
        /// Passes failed in a row, which sets how long the next wait is.
        failures: u32,
    },
    /// The server rejected the credential. Trying again cannot help and costs a failed login.
    NeedsSignIn { why: String },
    /// Failed for a reason no retry changes.
    Broken { why: String },
}

/// Whether a pass is the one that fills an account that has never been fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum First {
    /// Nothing has been fetched yet, so an empty list means "not yet" rather than "empty".
    Yes,
    /// The account has been fetched before.
    No,
}

/// How an account hears about new mail between passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Live {
    /// We ask on a timer.
    Polling,
    /// The server tells us (IMAP IDLE, JMAP push), so there is no timer.
    Pushed,
}

/// What a pass is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Connecting,
    Folders,
    Headers { mailbox: String },
    Flags,
    Bodies,
    Sending,
}

/// Why the server is being left alone for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// The connection could not be made.
    Unreachable,
    /// The server asked to be left alone, and named how long.
    Throttled,
    /// The server answered that it could not serve us now.
    ServerBusy,
}

/// Progress through a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count {
    pub done: u32,
    pub of: u32,
}

/// One folder a pass finished without.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trouble {
    /// The folder, when the failure was in one.
    pub mailbox: Option<String>,
    /// What to do about it.
    pub retry: Retry,
    /// What to tell the person.
    pub why: String,
}

/// What asked for a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The timer.
    Poll,
    /// The server said something changed.
    Push,
    /// A person pressed sync.
    Manual,
    /// A folder was opened.
    FolderOpen,
}

/// Something that happened to the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Start(Trigger),
    Stepped(Step, Option<Count>),
    Finished {
        trouble: Vec<Trouble>,
    },
    Failed {
        retry: Retry,
        why: String,
        pause: Pause,
    },
    Cancel,
    /// The wake the link asked for has come.
    Tick,
    /// The credential was replaced.
    SignedIn,
    /// The account started or stopped being pushed to.
    Live(Live),
}

/// What the caller must do after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    RunPass,
    CancelPass,
    /// Send [`Event::Tick`] at this time.
    WakeAt(DateTime<Utc>),
}

impl Link {
    /// Whether a request from `trigger` would start a pass.
    ///
    /// A broken account lets a person try again and nothing else: a timer must not keep knocking
    /// on a server that said no, but pressing sync is a decision.
    pub fn may_start(&self, trigger: Trigger) -> bool {
        match self {
            Link::Fresh | Link::Current { .. } | Link::Waiting { .. } => true,
            Link::Broken { .. } => trigger == Trigger::Manual,
            Link::Syncing { .. } | Link::NeedsSignIn { .. } => false,
        }
    }

    /// Whether a pass is running.
    pub fn is_busy(&self) -> bool {
        matches!(self, Link::Syncing { .. })
    }

    /// The state this link is at rest in: itself, or what a running pass will fall back to.
    pub fn resting(&self) -> &Link {
        match self {
            Link::Syncing { after, .. } => after.resting(),
            other => other,
        }
    }
}

/// How long to wait after `failures` failures in a row: the interval, doubled each time, up to
/// [`BACKOFF_CEILING`], and never less than the interval.
pub fn backoff(failures: u32, every: Duration) -> Duration {
    // Saturating on purpose: a machine left asleep for a week comes back to a shift count that
    // would otherwise wrap and produce a *short* wait.
    let doubling = 1u32
        .checked_shl(failures.saturating_sub(1).min(16))
        .unwrap_or(u32::MAX);
    every
        .saturating_mul(doubling)
        .min(BACKOFF_CEILING)
        .max(every)
}

/// What `event` does to `link` at `now`, given the poll interval `every`.
///
/// Pairs not in the table do nothing: the same state and no effects.
pub fn step(link: &Link, event: Event, now: DateTime<Utc>, every: Duration) -> (Link, Vec<Effect>) {
    match (link, event) {
        (Link::Syncing { .. }, event) => running(link, event, now, every),
        (Link::Fresh | Link::Current { .. } | Link::Waiting { .. }, Event::Start(_)) => begin(link),
        (Link::Broken { .. }, Event::Start(Trigger::Manual)) => begin(link),
        (
            Link::Current {
                at,
                live: Live::Polling,
                ..
            },
            Event::Tick,
        ) if now >= later(*at, every) => begin(link),
        (Link::Waiting { until, .. }, Event::Tick) if now >= *until => begin(link),
        (Link::NeedsSignIn { .. }, Event::SignedIn) => (
            Link::Syncing {
                first: First::No,
                step: Step::Connecting,
                count: None,
                after: Box::new(Link::Fresh),
            },
            vec![Effect::RunPass],
        ),
        (Link::Current { at, trouble, live }, Event::Live(now_live)) if *live != now_live => {
            let current = Link::Current {
                at: *at,
                trouble: trouble.clone(),
                live: now_live,
            };
            // Leaving push for polling puts the timer back; nothing else would wake the link.
            let wake = match now_live {
                Live::Polling => vec![Effect::WakeAt(later(*at, every).max(now))],
                Live::Pushed => vec![],
            };
            (current, wake)
        }
        _ => unchanged(link),
    }
}

fn unchanged(link: &Link) -> (Link, Vec<Effect>) {
    (link.clone(), vec![])
}

/// A pass starts from `from`, which is what a cancel returns to.
fn begin(from: &Link) -> (Link, Vec<Effect>) {
    let first = match from {
        Link::Fresh => First::Yes,
        _ => First::No,
    };
    let syncing = Link::Syncing {
        first,
        step: Step::Connecting,
        count: None,
        after: Box::new(from.clone()),
    };
    (syncing, vec![Effect::RunPass])
}

/// What an event does to a running pass.
fn running(link: &Link, event: Event, now: DateTime<Utc>, every: Duration) -> (Link, Vec<Effect>) {
    let Link::Syncing { first, after, .. } = link else {
        return unchanged(link);
    };
    match event {
        Event::Stepped(step, count) => {
            let moved = Link::Syncing {
                first: *first,
                step,
                count,
                after: after.clone(),
            };
            (moved, vec![])
        }
        Event::Finished { trouble } => {
            let live = live_of(after);
            let wake = match live {
                Live::Polling => vec![Effect::WakeAt(later(now, every))],
                Live::Pushed => vec![],
            };
            (
                Link::Current {
                    at: now,
                    trouble,
                    live,
                },
                wake,
            )
        }
        Event::Failed { retry, why, pause } => failed(after, retry, why, pause, now, every),
        Event::Cancel => ((**after).clone(), vec![Effect::CancelPass]),
        Event::Live(live) => {
            // Remembered for when the pass finishes, which is when it matters.
            let (rested, _) = step(after, Event::Live(live), now, every);
            let Link::Syncing { step, count, .. } = link else {
                return unchanged(link);
            };
            let kept = Link::Syncing {
                first: *first,
                step: step.clone(),
                count: *count,
                after: Box::new(rested),
            };
            (kept, vec![])
        }
        Event::Start(_) | Event::Tick | Event::SignedIn => unchanged(link),
    }
}

/// A pass failed. `after` is where it began, which holds the failures so far.
fn failed(
    after: &Link,
    retry: Retry,
    why: String,
    pause: Pause,
    now: DateTime<Utc>,
    every: Duration,
) -> (Link, Vec<Effect>) {
    let failures = match after {
        Link::Waiting { failures, .. } => failures.saturating_add(1),
        _ => 1,
    };
    let wait = |wait: Duration| {
        let until = later(now, wait);
        (
            Link::Waiting {
                until,
                why: pause,
                failures,
            },
            vec![Effect::WakeAt(until)],
        )
    };
    match retry {
        Retry::Now => wait(RETRY_NOW),
        Retry::After(named) => {
            let own = backoff(failures, every);
            match pause {
                // The wait is the server's to choose: capping it would turn "wait an hour"
                // into knocking twice inside it, and a rate limit is the refusal where
                // knocking early lengthens the lockout.
                Pause::Throttled => wait(named.max(own)),
                Pause::Unreachable | Pause::ServerBusy => wait(named.max(own).min(BACKOFF_CEILING)),
            }
        }
        Retry::NeedsReauth => (Link::NeedsSignIn { why }, vec![]),
        Retry::Fatal(fatal) => (
            Link::Broken {
                why: if fatal.is_empty() { why } else { fatal },
            },
            vec![],
        ),
    }
}

/// How the account was being told about mail before the running pass began.
fn live_of(after: &Link) -> Live {
    match after {
        Link::Current { live, .. } => *live,
        _ => Live::Polling,
    }
}

/// `now` plus `wait`, saturating at the end of time rather than panicking on a huge wait.
fn later(now: DateTime<Utc>, wait: Duration) -> DateTime<Utc> {
    TimeDelta::from_std(wait)
        .ok()
        .and_then(|delta| now.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests;
