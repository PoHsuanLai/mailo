//! What every engine of an account shares: the report of a pass, how a wait ends, and the few
//! helpers that settle an outbox entry.
//!
//! The IMAP and POP3 engine (`mail-engine`'s `AccountEngine`) and [`crate::jmap::JmapEngine`]
//! each run a pass and wait for the next; this is the vocabulary of both, so that neither
//! depends on the other.

use crate::RuntimeError;
use crate::error::Logged;
use chrono::{DateTime, Utc};
use mail_domain::{MailboxRef, MessageId, Retry};
use mail_store::{Settle, SqliteStore, Store};
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

/// How often a waiting watch looks in the outbox for a send that has come due, unless it is told
/// otherwise: local and cheap, one indexed query and no network.
pub const OUTBOX_LOOK: Duration = Duration::from_secs(15);

/// The longest one IMAP `IDLE` is held before it is ended and issued again.
///
/// RFC 2177 has a server drop an idle client after thirty minutes, and asks the client to
/// re-issue IDLE within twenty-nine. Twenty-five leaves room for a slow `DONE` and matches the
/// JMAP event stream's hold (`jmap::push`). It is also the longest a connection a NAT forgot can
/// go unnoticed while it is parked: the exchange that ends it fails, or the pass after it does.
pub const IDLE_REARM: Duration = Duration::from_secs(25 * 60);

/// What ended a wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woke {
    /// The server said something changed.
    Mail,
    /// Something in the outbox came due: a scheduled send, or one whose grace for Undo ran out.
    Due,
    /// The interval ran out, the server ended its IDLE without news, or the IDLE was held for
    /// [`IDLE_REARM`].
    Interval,
}

/// What a sync pass did, for the caller to log or show.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub headers_fetched: usize,
    pub bodies_fetched: usize,
    /// Attachments a sync had left on the server, fetched by this pass because the account is
    /// kept offline in full ([`AccountEngine::fetch_remote_parts`]).
    pub parts_fetched: usize,
    pub outbox_settled: usize,
    /// Messages handed to the submission server and accepted.
    pub submitted: usize,
    /// Operations that failed in a way worth surfacing rather than retrying.
    pub needs_attention: Vec<String>,
    /// Still queued when the pass ended, waiting on a retry.
    ///
    /// Separate from `needs_attention` because it is not trouble: a refused connection backs off
    /// and tries again, and interrupting the user about it would train them to ignore the
    /// warnings that matter. But it is not *nothing* either — someone who has just run `send`
    /// and then `sync` will otherwise read "0 sent" as success and believe their mail has gone.
    pub still_queued: usize,
    /// The server rejected the credential, on any account in this pass.
    ///
    /// Separate from `needs_attention`, which is prose for a person, because something has to
    /// *act* on it: a rejected password retried every five minutes is two hundred and eighty
    /// failed logins a day against the user's own mail server, which is how an account gets
    /// locked. A poll loop must stop on this and wait for the user, not back off and continue.
    pub needs_reauth: bool,
    /// The desktop's account service no longer lets Mail use the account (`Retry::NeedsGrant`):
    /// stopped on as `needs_reauth` is, and asked of the person as allowing, not signing in.
    pub needs_grant: bool,
    /// The longest wait a server asked for during this pass, where one asked.
    ///
    /// A rate limit is the one refusal whose entire remedy is doing nothing, and hammering
    /// through it lengthens the lockout — so the number the server named (or the hour
    /// `Retry::After` supplies when it names none) has to outlive the pass that saw it. Kept
    /// apart from `needs_reauth` because they call for opposite things: stop and ask the user,
    /// versus come back later unaided.
    pub hold: Option<std::time::Duration>,
    /// Messages this pass stored for the first time, in the order they were stored.
    ///
    /// Identities, not copies: by the time anyone reads this the same pass has applied the
    /// server's flags and may have fetched bodies, so what a caller wants to know about a message
    /// — is it still unread, is it still in the inbox — is the store's current answer, not the
    /// one the header fetch built. A message already held that arrives again (a remap, a body
    /// filling in, the same mail under a second UID) is not here: it did not *arrive*.
    pub arrived: Vec<MessageId>,
    /// Arrived messages a rule acted on, with the rules that did, in order.
    pub ruled: Vec<(MessageId, Vec<String>)>,
    /// Messages uploaded into a mailbox by this pass's outbox (`ProtoOp::Append`).
    pub appended: usize,
}

/// A draft's new send state, written down without letting the write fail the drain. A draft
/// deleted while its send was queued is nobody's concern; anything else the store says is logged.
pub fn note_state(written: Result<(), mail_store::StoreError>) {
    match written {
        Ok(()) | Err(mail_store::StoreError::NoDraft(_)) => {}
        Err(why) => log::warn!("a draft's send state was not recorded: {why}"),
    }
}

/// Refuse queued entry `id`, given up because a message it names was never found
/// ([`Dispatch::Lost`]): settled as [`Retry::Fatal`], so its undo puts back what
/// the server has, and said, so the person whose change it was is told it did not happen.
pub fn given_up(
    store: &dyn Store,
    id: mail_domain::OutboxId,
    reason: String,
    now: DateTime<Utc>,
    report: &mut SyncReport,
) -> Result<(), RuntimeError> {
    let retry = Retry::Fatal(reason.clone());
    report.saw(&retry);
    report.needs_attention.push(reason.clone());
    store.outbox_settle(id, Settle::Failed { reason, retry }, now)?;
    Ok(())
}

/// The messages a patch stored for the first time.
///
/// Only meaningful for the patch of a header fetch: that path upserts a message only when its
/// key is new, since a header carries no body to fill in. A body fetch's patch upserts messages
/// already held, and must not be read with this.
pub fn first_stored(patch: &mail_domain::Patch) -> impl Iterator<Item = MessageId> + '_ {
    patch.changes.iter().filter_map(|change| match change {
        mail_domain::Change::MessageUpsert(message) => Some(message.id),
        _ => None,
    })
}

impl SyncReport {
    /// Keep what a write that stopped part-way had already committed, and say the rest did not
    /// follow. True when `error` was such a stop: the pass then ends with what it has, so the
    /// messages that did arrive still meet the rules and the notifications, and the next pass
    /// writes the rest.
    pub fn stopped_by(&mut self, error: &RuntimeError) -> bool {
        let RuntimeError::Store(error) = error else {
            return false;
        };
        let Some(written) = error.written() else {
            return false;
        };
        self.arrived.extend(first_stored(written));
        self.needs_attention.push(error.to_string());
        true
    }

    /// Say that `count` messages of `mailbox` arrived and could not be read as messages. They
    /// were skipped, and the pass's cursor has moved past them, so nothing will fetch them
    /// again: the person is told rather than left with mail that silently never came.
    pub fn unreadable(&mut self, mailbox: &MailboxRef, count: usize) {
        if count > 0 {
            self.needs_attention.push(format!(
                "{count} message(s) in {} could not be read and were skipped",
                mailbox.path
            ));
        }
    }

    /// Fold in the pass that followed this one, for a drain that queued work for itself.
    /// What is still queued is the later pass's word, since it saw the queue last.
    pub fn absorb(&mut self, later: SyncReport) {
        self.outbox_settled += later.outbox_settled;
        self.submitted += later.submitted;
        self.appended += later.appended;
        self.needs_attention.extend(later.needs_attention);
        self.still_queued = later.still_queued;
        self.needs_reauth |= later.needs_reauth;
        self.needs_grant |= later.needs_grant;
        self.hold = match (self.hold, later.hold) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    /// Fold one operation's verdict into the pass.
    ///
    /// The two facts a caller has to *act* on, gathered in one place so the several sites that
    /// see a `Retry` cannot disagree about which ones matter. `Retry::Now` and `Retry::Fatal`
    /// are decided within the pass — reconnect, or give up on that operation — and leave nothing
    /// for the caller to schedule.
    pub fn saw(&mut self, retry: &Retry) {
        match retry {
            Retry::NeedsReauth => self.needs_reauth = true,
            Retry::NeedsGrant => self.needs_grant = true,
            // The longest wait anyone asked for. Ordinary backoffs land here too, at a minute or
            // so, and are absorbed by the caller's own interval; a rate limit is an hour and is
            // not. Both mean the same thing to a scheduler, so neither needs a special case.
            Retry::After(wait) => self.hold = Some(self.hold.map_or(*wait, |had| had.max(*wait))),
            Retry::Now | Retry::Fatal(_) => {}
        }
    }
}

/// Resolves when a never-tried outbox entry that was not yet due at `after` becomes due.
///
/// Looks again every `every`, so a send queued after the wait began is noticed; between looks it
/// sleeps until the earliest time it knows of, so a known one is not late by the interval.
pub async fn due_alarm(
    store: Arc<SqliteStore>,
    account: AccountId,
    after: DateTime<Utc>,
    every: Duration,
) {
    loop {
        let now = Utc::now();
        let next = store
            .outbox_next(account.clone(), after)
            .or_log("the outbox's next attempt could not be read")
            .flatten();
        if next.is_some_and(|at| at <= now) {
            return;
        }
        let until = next
            .and_then(|at| (at - now).to_std().ok())
            .map_or(every, |left| left.min(every));
        tokio::time::sleep(until).await;
    }
}
