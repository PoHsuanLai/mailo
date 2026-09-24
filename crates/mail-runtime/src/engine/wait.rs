//! Waiting between passes, and coming back early for a send that has come due.
//!
//! A watch spends almost all of its life here: parked in IDLE, or asleep for a poll interval.
//! Either can last half an hour, and a send the user scheduled for nine o'clock that left at
//! twenty-five past because the server had nothing to say in between is a send that did not
//! happen when it was asked to. So the wait is a race between the server and the outbox, and
//! the outbox side is local: a query against the store, never a connection.

use super::AccountEngine;
use crate::{Cancel, RuntimeError};
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Incoming, MailboxRef, WatchMode};
use mail_proto::Backend;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// What ended a wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woke {
    /// The server said something changed.
    Mail,
    /// Something in the outbox came due: a scheduled send, or one whose grace for Undo ran out.
    Due,
    /// The interval ran out, or the server ended its IDLE without news.
    Interval,
}

impl<B: Backend> AccountEngine<B> {
    /// Wait for new mail, the way the server prefers, or until the outbox has something due.
    ///
    /// IDLE where the server offers it, raced against the outbox; `poll` otherwise, and after an
    /// IDLE that ended without news. When the outbox wins an IDLE, the session is interrupted in
    /// protocol — `DONE` rather than a dropped socket — through the same path as cancellation.
    ///
    /// Only sends queued before the wait began are known by time; one queued later, by another
    /// process, is noticed within [`super::Schedule::outbox`]. Reads the wall clock, as a watch
    /// must: it outlives any `now` it could be handed.
    pub async fn wait(
        &mut self,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        poll: Duration,
    ) -> Result<Woke, RuntimeError> {
        let started = Utc::now();
        let (store, account, every) = (self.store.clone(), self.account, self.schedule.outbox);
        let alarm = || due_alarm(store.clone(), account, started, every);

        // An account that keeps its mail here has no server to park in IDLE on, whatever its
        // stored capabilities say: it only ever sleeps.
        let server = !matches!(self.plan.incoming, Incoming::Local);
        if server && matches!(self.caps().watch, WatchMode::Idle) {
            let alarm = alarm();
            let (tx, mut inner) = watch::channel(false);
            let watched = self.watch(mailbox, &mut inner);
            tokio::pin!(watched, alarm);
            let mut woke = None;
            let result = loop {
                tokio::select! {
                    result = &mut watched => break result,
                    () = &mut alarm, if woke.is_none() => {
                        woke = Some(Woke::Due);
                        let _ = tx.send(true);
                    }
                    changed = cancel.changed(), if woke.is_none() => {
                        if changed.is_err() || *cancel.borrow() {
                            woke = Some(Woke::Interval);
                            let _ = tx.send(true);
                        }
                    }
                }
            };
            match (woke, result) {
                // Interrupted for the outbox. However the session wound down — a clean `DONE`, or
                // abandoned after one more exchange — the send is what matters now.
                (Some(Woke::Due), Ok(_) | Err(RuntimeError::Cancelled)) => return Ok(Woke::Due),
                (Some(_), Ok(_)) => return Err(RuntimeError::Cancelled),
                (_, Err(e)) => return Err(e),
                (None, Ok(true)) => return Ok(Woke::Mail),
                // IDLE ended without news: the poll interval below is the floor.
                (None, Ok(false)) => {}
            }
        }

        tokio::select! {
            () = tokio::time::sleep(poll) => Ok(Woke::Interval),
            () = alarm() => Ok(Woke::Due),
            // A closed sender means the owner went away, which `drive` treats as cancellation too.
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    Err(RuntimeError::Cancelled)
                } else {
                    Ok(Woke::Interval)
                }
            }
        }
    }
}

/// Resolves when a never-tried outbox entry that was not yet due at `after` becomes due.
///
/// Looks again every `every`, so a send queued after the wait began is noticed; between looks it
/// sleeps until the earliest time it knows of, so a known one is not late by the interval.
pub(crate) async fn due_alarm(
    store: Arc<SqliteStore>,
    account: AccountId,
    after: DateTime<Utc>,
    every: Duration,
) {
    loop {
        let now = Utc::now();
        let next = store.outbox_next(account, after).ok().flatten();
        if next.is_some_and(|at| at <= now) {
            return;
        }
        let until = next
            .and_then(|at| (at - now).to_std().ok())
            .map_or(every, |left| left.min(every));
        tokio::time::sleep(until).await;
    }
}
