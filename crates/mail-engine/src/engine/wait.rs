//! Waiting between passes, and coming back early for a send that has come due.
//!
//! A watch spends almost all of its life here: parked in IDLE, or asleep for a poll interval.
//! Either can last half an hour, and a send the user scheduled for nine o'clock that left at
//! twenty-five past because the server had nothing to say in between is a send that did not
//! happen when it was asked to. So the wait is a race between the server and the outbox, and
//! the outbox side is local: a query against the store, never a connection.

use super::{AccountEngine, Notify};
use chrono::Utc;
use mail_domain::{MailboxRef, WatchMode};
use mail_proto::Backend;
use mail_runtime::pass::due_alarm;
use mail_runtime::{Cancel, RuntimeError};
use mail_runtime::{IDLE_REARM, Woke};
use std::time::Duration;
use tokio::sync::watch;

/// Why a held IDLE was ended from this side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The outbox has something due.
    Due,
    /// It was held for [`IDLE_REARM`].
    Rearm,
    /// The caller asked the wait to stop.
    Cancelled,
}

impl<B: Backend> AccountEngine<B> {
    /// Wait for new mail, the way the server prefers, or until the outbox has something due.
    ///
    /// IDLE where the server offers it, raced against the outbox and held for at most
    /// [`IDLE_REARM`]; `poll` otherwise, and after an IDLE the server ended without news. When the
    /// outbox or the re-arm wins an IDLE, the session is interrupted in protocol — `DONE` rather
    /// than a dropped socket — through the same path as cancellation.
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
        let (store, account, every) = (
            self.store.clone(),
            self.account.clone(),
            self.schedule.outbox,
        );
        let alarm = || due_alarm(store.clone(), account.clone(), started, every);
        let learning = self.notifies() == Notify::Unasked;

        // An account that keeps its mail here has no server to park in IDLE on, whatever its
        // stored capabilities say: it only ever sleeps.
        let server = self.plan.incoming.caps().has_server;
        if server && matches!(self.caps().watch, WatchMode::Idle) {
            let alarm = alarm();
            // IDLE is not held for ever: past `IDLE_REARM` it is ended in protocol and the wait
            // returns, so the next one issues it again on a fresh connection, and the pass in
            // between proves the account is reachable rather than assuming it.
            let rearm = tokio::time::sleep(IDLE_REARM);
            let (tx, mut inner) = watch::channel(false);
            // Scoped, so the watch lets go of the engine before the arms below read it.
            let (ended, result) = {
                let watched = self.watch(mailbox, &mut inner);
                tokio::pin!(watched, alarm, rearm);
                let mut ended = None;
                let result = loop {
                    tokio::select! {
                        result = &mut watched => break result,
                        () = &mut alarm, if ended.is_none() => {
                            ended = Some(Ended::Due);
                            let _ = tx.send(true);
                        }
                        () = &mut rearm, if ended.is_none() => {
                            ended = Some(Ended::Rearm);
                            // The watch ends on its own once it is gone; nothing waits on this send.
                            let _ = tx.send(true);
                        }
                        changed = cancel.changed(), if ended.is_none() => {
                            if changed.is_err() || *cancel.borrow() {
                                ended = Some(Ended::Cancelled);
                                let _ = tx.send(true);
                            }
                        }
                    }
                };
                (ended, result)
            };
            match (ended, result) {
                // Interrupted for the outbox. However the session wound down — a clean `DONE`, or
                // abandoned after one more exchange — the send is what matters now.
                (Some(Ended::Due), Ok(_) | Err(RuntimeError::Cancelled)) => return Ok(Woke::Due),
                // Held as long as IDLE should be, and ended for it: a look at the account, as a
                // JMAP stream that reaches its hold is. No floor: this is not a busy mailbox.
                (Some(Ended::Rearm), Ok(_) | Err(RuntimeError::Cancelled)) => {
                    return Ok(Woke::Interval);
                }
                (Some(Ended::Cancelled), Ok(_)) => return Err(RuntimeError::Cancelled),
                (_, Err(e)) => return Err(e),
                (None, Ok(true)) => return Ok(Woke::Mail),
                // The server has no `NOTIFY`, which this watch has just found out: the caller
                // watches the other mailboxes itself, and is told to begin now rather than after
                // an interval.
                (None, Ok(false)) if learning && self.notifies() == Notify::Absent => {
                    return Ok(Woke::Interval);
                }
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
