//! Messages a search of the server brought here, and messages by server address. Kept in step
//! with `memory/found.rs`, which `tests/found.rs` holds it to.

use super::SqliteStore;
use super::row::uuid;
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{MessageId, RemoteRef, ThreadId};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    pub(super) fn write_found(
        &self,
        account: AccountId,
        messages: &[MessageId],
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        for message in messages {
            let held: Option<i64> = db
                .query_row(
                    "SELECT 1 FROM messages WHERE id = ?1 AND account = ?2",
                    params![message.to_string(), account.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if held.is_none() {
                return Err(StoreError::NoMessage(*message));
            }
            // The first search to bring it stands; a later one found it here.
            db.execute(
                "INSERT OR IGNORE INTO found_on_server (message, account, found_at)
                 VALUES (?1, ?2, ?3)",
                params![message.to_string(), account.to_string(), now.to_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn read_found_in(&self, threads: &[ThreadId]) -> Result<Vec<ThreadId>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT 1 FROM found_on_server f JOIN messages m ON m.id = f.message
             WHERE m.thread = ?1 LIMIT 1",
        )?;
        let mut out: Vec<ThreadId> = Vec::new();
        for thread in threads {
            if out.contains(thread) {
                continue;
            }
            if stmt
                .query_row([thread.to_string()], |r| r.get::<_, i64>(0))
                .optional()?
                .is_some()
            {
                out.push(*thread);
            }
        }
        Ok(out)
    }

    pub(super) fn read_held_at(
        &self,
        account: AccountId,
        remotes: &[RemoteRef],
    ) -> Result<Vec<(RemoteRef, MessageId)>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT message FROM remote_map WHERE account = ?1 AND mailbox = ?2
             AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
        )?;
        let mut out = Vec::new();
        for remote in remotes {
            let (mailbox, uidvalidity, uid, uidl) = crate::remote_row::columns(remote);
            let found: Option<String> = stmt
                .query_row(
                    params![account.to_string(), mailbox, uidvalidity, uid, uidl],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = found {
                out.push((
                    remote.clone(),
                    MessageId::from_uuid(uuid("MessageId", &id)?),
                ));
            }
        }
        Ok(out)
    }
}
