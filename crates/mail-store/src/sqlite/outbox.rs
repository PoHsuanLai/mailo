//! The outbox: resolving remote intent, queueing it, and settling it.
//!
//! This module is where FINDINGS.md F12 lands. `Op::apply` states a [`RemoteIntent`] in local
//! ids because it cannot do otherwise — `RemoteRef` needs `remote_map`'s many-to-one mapping,
//! and `ProtoOp::SetLabels` needs server-side label names. Both tables exist here and nowhere
//! else, so this is the only place the resolution can happen.

use super::SqliteStore;
use super::row::{from_time, json, to_json};
use crate::{OutboxEntry, Settle, StoreError};
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::{
    AccountId, Change, Membership, MessageId, OutboxId, Patch, ProtoOp, RemoteIntent, RemoteRef,
    Retry,
};
use rusqlite::params;

/// Backoff for a retryable failure: 1s, 2s, 4s … capped at an hour.
///
/// Capped rather than unbounded because the common cause is a laptop lid, and a user who opens
/// it after a long flight should not wait a day for their mail.
fn backoff(attempts: u32) -> TimeDelta {
    let secs = 1i64 << attempts.min(12);
    TimeDelta::try_seconds(secs.min(3600)).unwrap_or_else(|| TimeDelta::try_seconds(60).unwrap())
}

impl SqliteStore {
    /// Every way the server can address these messages.
    ///
    /// Several per message is normal and correct: a Gmail message marked read must be marked
    /// read in INBOX *and* in All Mail, or the next sync reports it unread again.
    fn refs_for(
        &self,
        account: AccountId,
        messages: &[MessageId],
    ) -> Result<Vec<RemoteRef>, StoreError> {
        let mut out = Vec::new();
        let mut stmt = self.db.prepare_cached(
            "SELECT mailbox, uidvalidity, uid, uidl FROM remote_map
             WHERE account = ?1 AND message = ?2",
        )?;
        for id in messages {
            let rows = stmt.query_map(params![account.to_string(), id.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (mailbox, uidvalidity, uid, uidl) = row?;
                out.push(match (uid, uidl) {
                    (Some(uid), None) => RemoteRef::Imap {
                        mailbox,
                        uidvalidity: uidvalidity.unwrap_or(0) as u32,
                        uid: uid as u32,
                    },
                    (None, Some(uidl)) => RemoteRef::Pop { uidl },
                    _ => {
                        return Err(StoreError::Decode {
                            what: "remote_map row".to_owned(),
                            why: "row has neither a uid nor a uidl".to_owned(),
                        });
                    }
                });
            }
        }
        Ok(out)
    }

    fn label_names(&self, labels: &[mail_domain::LabelId]) -> Result<Vec<String>, StoreError> {
        let mut out = Vec::new();
        let mut stmt = self
            .db
            .prepare_cached("SELECT name FROM labels WHERE id = ?1")?;
        for id in labels {
            let name: Option<String> = stmt.query_row(params![id.to_string()], |r| r.get(0)).ok();
            // A label we do not know is not an error: it may have been deleted between the
            // user's click and the outbox draining. Sending an unknown name would be worse.
            if let Some(name) = name {
                out.push(name);
            }
        }
        Ok(out)
    }

    /// Turn intent into a wire operation, or `None` when there is nothing to say.
    pub(super) fn resolve_intent(
        &self,
        account: AccountId,
        intent: &RemoteIntent,
    ) -> Result<Option<ProtoOp>, StoreError> {
        let messages = match intent {
            RemoteIntent::SetFlags { messages, .. }
            | RemoteIntent::SetMailbox { messages, .. }
            | RemoteIntent::SetLabels { messages, .. } => messages,
        };
        let remotes = self.refs_for(account, messages)?;
        if remotes.is_empty() {
            // Normal, not exceptional: a message composed locally and not yet sent has no
            // server address at all.
            return Ok(None);
        }
        Ok(Some(match intent {
            RemoteIntent::SetFlags { read, star, .. } => ProtoOp::SetFlags {
                remotes,
                read: *read,
                star: *star,
            },
            RemoteIntent::SetMailbox { role, .. } => ProtoOp::SetMailbox {
                remotes,
                role: *role,
            },
            RemoteIntent::SetLabels { add, remove, .. } => ProtoOp::SetLabels {
                remotes,
                add: self.label_names(add)?,
                remove: self.label_names(remove)?,
            },
        }))
    }

    /// The local changes this intent corresponds to, per message.
    ///
    /// Derived from the intent rather than taken from the caller's forward patch, because the
    /// intent is exactly "what the server is being asked to do" — which is precisely what has
    /// to be re-layered on top of server truth until the server agrees.
    fn pending_of(intent: &RemoteIntent) -> Vec<(MessageId, Vec<Change>)> {
        match intent {
            RemoteIntent::SetFlags {
                messages,
                read,
                star,
            } => messages
                .iter()
                .map(|m| {
                    let mut changes = Vec::new();
                    if let Some(read) = read {
                        changes.push(Change::MessageRead(*m, *read));
                    }
                    if let Some(star) = star {
                        changes.push(Change::MessageStar(*m, *star));
                    }
                    (*m, changes)
                })
                .collect(),
            RemoteIntent::SetMailbox { messages, role } => messages
                .iter()
                .map(|m| (*m, vec![Change::MessageMailbox(*m, *role)]))
                .collect(),
            RemoteIntent::SetLabels {
                messages,
                add,
                remove,
            } => messages
                .iter()
                .map(|m| {
                    let mut changes = Vec::new();
                    for l in add {
                        changes.push(Change::MessageLabel(*m, *l, Membership::In));
                    }
                    for l in remove {
                        changes.push(Change::MessageLabel(*m, *l, Membership::Out));
                    }
                    (*m, changes)
                })
                .collect(),
        }
    }

    pub(super) fn queue(
        &self,
        account: AccountId,
        intent: RemoteIntent,
        undo: &Patch,
        now: DateTime<Utc>,
    ) -> Result<Option<OutboxId>, StoreError> {
        let Some(op) = self.resolve_intent(account, &intent)? else {
            return Ok(None);
        };
        let tx = self.db.unchecked_transaction()?;
        self.db.execute(
            "INSERT INTO outbox (account, op, undo, attempts, next_attempt, created_at)
             VALUES (?1, ?2, ?3, 0, ?4, ?4)",
            params![
                account.to_string(),
                to_json("ProtoOp", &op)?,
                to_json("Patch", undo)?,
                from_time(now),
            ],
        )?;
        let id = OutboxId::from_i64(self.db.last_insert_rowid());
        for (message, changes) in Self::pending_of(&intent) {
            if changes.is_empty() {
                continue;
            }
            self.db.execute(
                "INSERT OR REPLACE INTO pending_changes (message, outbox, changes)
                 VALUES (?1, ?2, ?3)",
                params![
                    message.to_string(),
                    id.as_i64(),
                    to_json("pending changes", &changes)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(Some(id))
    }

    pub(super) fn due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError> {
        // Insertion order, not next_attempt order. Two operations on one thread must reach the
        // server in the order the user performed them, or the result is whichever won the race.
        let mut stmt = self.db.prepare_cached(
            "SELECT id, op, undo, attempts, next_attempt FROM outbox
             WHERE account = ?1 AND next_attempt <= ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![account.to_string(), from_time(now)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, op, undo, attempts, next) = row?;
            out.push(OutboxEntry {
                id: OutboxId::from_i64(id),
                account,
                op: json("ProtoOp", &op)?,
                undo: json("Patch", &undo)?,
                attempts: attempts as u32,
                next_attempt: super::row::time("next_attempt", &next)?,
            });
        }
        Ok(out)
    }

    pub(super) fn settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let tx = self.db.unchecked_transaction()?;
        match settle {
            Settle::Ok => {
                // Confirmed. The local value and the server's now agree, so it is no longer
                // pending and must not be re-layered over the next ingest.
                self.drop_entry(id)?;
            }
            Settle::Failed { reason, retry } => match retry {
                Retry::Now | Retry::After(_) => {
                    let attempts: i64 = self.db.query_row(
                        "SELECT attempts FROM outbox WHERE id = ?1",
                        params![id.as_i64()],
                        |r| r.get(0),
                    )?;
                    let floor = match retry {
                        Retry::After(d) => TimeDelta::from_std(d).unwrap_or(TimeDelta::zero()),
                        _ => TimeDelta::zero(),
                    };
                    let wait = backoff(attempts as u32).max(floor);
                    self.db.execute(
                        "UPDATE outbox SET attempts = attempts + 1, next_attempt = ?2,
                             last_error = ?3 WHERE id = ?1",
                        params![id.as_i64(), from_time(now + wait), reason],
                    )?;
                }
                Retry::NeedsReauth => {
                    // Keep it queued and keep it pending: the user's change is not wrong, the
                    // credential is. Park it far enough out that it is not retried in a loop,
                    // and let the runtime surface the reauth prompt.
                    self.db.execute(
                        "UPDATE outbox SET next_attempt = ?2, last_error = ?3 WHERE id = ?1",
                        params![
                            id.as_i64(),
                            from_time(now + TimeDelta::try_hours(24).expect("24h is in range")),
                            reason
                        ],
                    )?;
                }
                Retry::Fatal(_) => {
                    // The server will refuse this forever. Undo the optimistic local change,
                    // or the user is left looking at a state that will never become true.
                    let undo: String = self.db.query_row(
                        "SELECT undo FROM outbox WHERE id = ?1",
                        params![id.as_i64()],
                        |r| r.get(0),
                    )?;
                    let patch: Patch = json("Patch", &undo)?;
                    self.drop_entry(id)?;
                    for change in &patch.changes {
                        if let Some(thread) = self.write_change(change)? {
                            self.refresh_summary(thread)?;
                        }
                    }
                }
            },
        }
        tx.commit()?;
        Ok(())
    }

    fn drop_entry(&self, id: OutboxId) -> Result<(), StoreError> {
        // pending_changes cascades on the foreign key, but be explicit: a stale pending row
        // would be silently re-applied over every future ingest.
        self.db.execute(
            "DELETE FROM pending_changes WHERE outbox = ?1",
            params![id.as_i64()],
        )?;
        self.db
            .execute("DELETE FROM outbox WHERE id = ?1", params![id.as_i64()])?;
        Ok(())
    }
}
