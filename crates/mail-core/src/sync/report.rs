//! What a sync pass found, kept as data.
//!
//! A pass used to be flattened to prose as soon as it ended: each account's report became a few
//! lines of text, a failed one became "address: why", and what the caller needed to *act* on was
//! a pair of fields pulled out on the way (FINDINGS F147). The window wants to say which account
//! is waiting on a sign-in, which is backing off and for how long, and which folder is at fault,
//! none of which can be read back out of a sentence. So the structure survives here, and the
//! prose is rendered from it ([`prose`]) by the callers that want prose.

use std::collections::BTreeMap;
use std::time::Duration;

use mail_domain::{AccountId, MessageId, Retry, Retryable};
use mail_proto::ProtoError;
use mail_runtime::{RuntimeError, SyncReport};
use tokio::sync::watch;

use crate::fetch::Pause;

/// What one account's pass fetched and settled. [`SyncReport`] without its trouble, which is
/// kept as [`Trouble`] so that each piece of it carries its decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub headers_fetched: usize,
    pub bodies_fetched: usize,
    /// Attachments kept offline by this pass.
    pub parts_fetched: usize,
    pub outbox_settled: usize,
    /// Messages handed to the submission server and accepted.
    pub submitted: usize,
    /// Messages uploaded into a mailbox by the outbox.
    pub appended: usize,
    /// Still queued when the pass ended, waiting on a retry.
    pub still_queued: usize,
    /// Messages this pass stored for the first time, in order.
    pub arrived: Vec<MessageId>,
    /// Arrived messages a rule acted on, with the rules that did.
    pub ruled: Vec<(MessageId, Vec<String>)>,
}

/// One thing that went wrong in a pass that otherwise ran, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trouble {
    /// The mailbox it happened in. `None` for a step that belongs to the account, such as the
    /// rules, the outbox or the folder list.
    pub mailbox: Option<String>,
    /// The decision: wait, ask the user to sign in, or give up on it.
    pub retry: Retry,
    /// What a person is told, as it has always been worded (the mailbox path is already in it
    /// where there is one). `None` when the failure is classified but has nothing to add to a
    /// line the same pass already said, such as an outbox that was refused its credential.
    pub why: Option<String>,
}

/// What a pass found on one account that reached the server and ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountReport {
    pub account: AccountId,
    pub address: String,
    pub counts: Counts,
    pub trouble: Vec<Trouble>,
}

/// How one account's pass ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassEnd {
    /// It ran. Some of it may still have gone wrong: see [`AccountReport::trouble`].
    Finished(AccountReport),
    /// It could not run at all: no credential, no connection, a refused sign-in.
    Failed {
        account: AccountId,
        address: String,
        retry: Retry,
        why: String,
        /// Why the server is to be left alone, when `retry` says to wait.
        pause: Pause,
    },
    /// The caller's cancel signal ended it.
    Cancelled { account: AccountId, address: String },
}

/// A pass's failure to run, before it knows which account it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub retry: Retry,
    pub why: String,
    /// What kind of wait `retry` asks for. Said only where the error is classified: the delay
    /// alone cannot tell a rate limit from a dropped connection.
    pub pause: Pause,
}

/// An error that knows why a wait after it is a wait.
pub(crate) trait Pauses {
    /// What the server did, as far as this error can tell.
    fn pause(&self) -> Pause;
}

impl Pauses for RuntimeError {
    fn pause(&self) -> Pause {
        match self {
            RuntimeError::Connect(_)
            | RuntimeError::Io(_)
            | RuntimeError::Tls(_)
            | RuntimeError::Proto(ProtoError::UnexpectedEof) => Pause::Unreachable,
            RuntimeError::Proto(ProtoError::Throttled { .. }) => Pause::Throttled,
            // A transient refusal is the server saying "not now". Everything else that waits is
            // a server that answered, and answered badly. Graph folds a `429` in with `503`
            // (`graph::refused`), so the two cannot be told apart here and are not guessed at.
            _ => Pause::ServerBusy,
        }
    }
}

/// A store failure is ours, not the server's, and none of the three says so. A wait after one is
/// the same wait, so it takes the mildest.
impl Pauses for mail_store::StoreError {
    fn pause(&self) -> Pause {
        Pause::ServerBusy
    }
}

/// Where a pass is, for a window to show. Emitted as each step starts and as it finishes; a
/// step the server gives no total for has `of: None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Connecting,
    Folders,
    Headers {
        mailbox: String,
        done: u32,
        of: Option<u32>,
    },
    Flags {
        mailbox: String,
    },
    Bodies {
        done: u32,
        of: Option<u32>,
    },
    Sending,
}

/// Where one account's pass reports progress.
pub(crate) type Emit<'a> = Option<&'a dyn Fn(Progress)>;

/// What a caller can hand a pass beyond the accounts it is to run.
#[derive(Default)]
pub struct Hooks<'a> {
    /// Told of each account's progress as the pass goes.
    pub progress: Option<&'a dyn Fn(AccountId, Progress)>,
    /// A signal per account that ends its pass. An account with none runs to the end.
    pub cancel: BTreeMap<AccountId, watch::Receiver<bool>>,
}

impl std::fmt::Debug for Hooks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks")
            .field("progress", &self.progress.map(|_| "a sink"))
            .field("cancel", &self.cancel.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Failure {
    pub fn fatal(why: impl Into<String>) -> Self {
        let why = why.into();
        Failure {
            retry: Retry::Fatal(why.clone()),
            why,
            pause: Pause::ServerBusy,
        }
    }

    /// A credential that is missing or refused: nothing to wait for, so the pause is not read.
    pub fn reauth(why: impl Into<String>) -> Self {
        Failure {
            retry: Retry::NeedsReauth,
            why: why.into(),
            pause: Pause::ServerBusy,
        }
    }

    /// A typed error, with `prefix` before what it says.
    pub fn of<E: Retryable + Pauses + std::fmt::Display>(prefix: &str, e: &E) -> Self {
        Failure {
            retry: e.retry(),
            why: format!("{prefix}{e}"),
            pause: e.pause(),
        }
    }
}

/// What the passes of one account's steps gathered: [`Counts`] and the trouble so far.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Done {
    pub counts: Counts,
    pub trouble: Vec<Trouble>,
}

impl Done {
    /// Record a failure, worded for a person.
    pub fn trouble(&mut self, mailbox: Option<&str>, retry: Retry, why: String) {
        self.trouble.push(Trouble {
            mailbox: mailbox.map(str::to_owned),
            retry,
            why: Some(why),
        });
    }

    /// Record a note an engine worded itself. It carries no classification of its own: it is an
    /// operation given up on, which is what the engine surfaces rather than retries.
    pub fn notes(&mut self, mailbox: Option<&str>, notes: Vec<String>) {
        for note in notes {
            self.trouble(mailbox, Retry::Fatal(note.clone()), note);
        }
    }

    /// Record the refusals an engine's own report carries as flags.
    pub fn flags(&mut self, report: &SyncReport) {
        if report.needs_reauth {
            self.trouble.push(Trouble {
                mailbox: None,
                retry: Retry::NeedsReauth,
                why: None,
            });
        }
        if let Some(wait) = report.hold {
            self.trouble.push(Trouble {
                mailbox: None,
                retry: Retry::After(wait),
                why: None,
            });
        }
    }

    /// An engine's whole report, as data.
    pub fn from_report(report: SyncReport) -> Self {
        let mut done = Done {
            counts: Counts {
                headers_fetched: report.headers_fetched,
                bodies_fetched: report.bodies_fetched,
                parts_fetched: report.parts_fetched,
                outbox_settled: report.outbox_settled,
                submitted: report.submitted,
                appended: report.appended,
                still_queued: report.still_queued,
                arrived: report.arrived.clone(),
                ruled: report.ruled.clone(),
            },
            trouble: Vec::new(),
        };
        done.flags(&report);
        done.notes(None, report.needs_attention);
        done
    }

    /// The old shape, for the callers that read prose.
    pub fn to_report(&self) -> SyncReport {
        let c = &self.counts;
        SyncReport {
            headers_fetched: c.headers_fetched,
            bodies_fetched: c.bodies_fetched,
            parts_fetched: c.parts_fetched,
            outbox_settled: c.outbox_settled,
            submitted: c.submitted,
            appended: c.appended,
            still_queued: c.still_queued,
            arrived: c.arrived.clone(),
            ruled: c.ruled.clone(),
            needs_attention: said(&self.trouble),
            needs_reauth: needs_reauth(&self.trouble),
            hold: hold(&self.trouble),
        }
    }

    pub fn of(self, account: &super::Configured) -> AccountReport {
        AccountReport {
            account: account.id,
            address: account.address.clone(),
            counts: self.counts,
            trouble: self.trouble,
        }
    }
}

/// The lines trouble says, in the order it happened.
fn said(trouble: &[Trouble]) -> Vec<String> {
    trouble.iter().filter_map(|t| t.why.clone()).collect()
}

/// Whether any of `trouble` is a refused credential.
pub(crate) fn needs_reauth(trouble: &[Trouble]) -> bool {
    trouble
        .iter()
        .any(|t| matches!(t.retry, Retry::NeedsReauth))
}

/// The longest wait any of `trouble` asked for.
pub(crate) fn hold(trouble: &[Trouble]) -> Option<Duration> {
    trouble
        .iter()
        .filter_map(|t| match t.retry {
            Retry::After(wait) => Some(wait),
            _ => None,
        })
        .max()
}

impl PassEnd {
    pub fn account(&self) -> AccountId {
        match self {
            PassEnd::Finished(report) => report.account,
            PassEnd::Failed { account, .. } | PassEnd::Cancelled { account, .. } => *account,
        }
    }

    /// Whether the pass may have written to the store, so that a reader of it should look again.
    ///
    /// A pass that ran may have stored mail, flags or settled sends, and the counts do not say
    /// all of that (a flag sweep is not counted), so any pass that ran says yes. One that could
    /// not run at all stored nothing. A cancelled one may have stored part of what it began.
    pub fn may_have_stored(&self) -> bool {
        match self {
            PassEnd::Finished(_) | PassEnd::Cancelled { .. } => true,
            PassEnd::Failed { .. } => false,
        }
    }

    /// Whether this account's credential was refused. A poll loop must stop on it.
    pub fn needs_reauth(&self) -> bool {
        match self {
            PassEnd::Finished(report) => needs_reauth(&report.trouble),
            PassEnd::Failed { retry, .. } => matches!(retry, Retry::NeedsReauth),
            PassEnd::Cancelled { .. } => false,
        }
    }

    /// The longest wait this account's server asked for.
    pub fn hold(&self) -> Option<Duration> {
        match self {
            PassEnd::Finished(report) => hold(&report.trouble),
            PassEnd::Failed {
                retry: Retry::After(wait),
                ..
            } => Some(*wait),
            PassEnd::Failed { .. } | PassEnd::Cancelled { .. } => None,
        }
    }
}

/// How one account's pass is told to a person: what `sync` and `watch` print.
pub fn prose(end: &PassEnd, now: chrono::DateTime<chrono::Utc>) -> String {
    match end {
        PassEnd::Finished(report) => {
            let done = Done {
                counts: report.counts.clone(),
                trouble: report.trouble.clone(),
            };
            super::one_line(&report.address, &done.to_report(), now)
        }
        PassEnd::Failed { address, why, .. } => format!("{address}: {why}\n"),
        PassEnd::Cancelled { address, .. } => format!("{address}: cancelled\n"),
    }
}

/// How a pass over several accounts reads: their prose in order, and the two facts a loop acts
/// on.
pub fn summarise(ends: &[PassEnd], now: chrono::DateTime<chrono::Utc>) -> super::Ran {
    super::Ran {
        text: ends.iter().map(|end| prose(end, now)).collect(),
        rejected: ends.iter().any(PassEnd::needs_reauth),
        // Only from an account that ran. A connection that would not open also says "after five
        // seconds", and read as a rate limit it would end the loop's backing-off: `Throttled`
        // resets the failure count, so a server that is down would be polled at the flat
        // interval for ever. Such a failure has always been the loop's own to back off from.
        hold: ends
            .iter()
            .filter(|end| matches!(end, PassEnd::Finished(_)))
            .filter_map(PassEnd::hold)
            .max(),
    }
}

mod event;
pub use event::outcome;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod tests;
