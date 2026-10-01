//! Addresses of messages the user deleted forever, until the server no longer lists them.
//!
//! Migration 0025 says why they are kept: a destroyed message is removed here at once, and its
//! `remote_map` rows go with it, but the server lists it until the deletion reaches it. Counted as
//! held ([`crate::Store::remote_refs`]), those addresses keep a sync from fetching the message
//! back into Trash, and the queued deletion is sent to them. `memory/destroyed.rs` keeps the same
//! rules for the in-memory store.

use super::SqliteStore;
use crate::StoreError;
use crate::dispatch::Answered;
use mail_domain::{AccountId, MailboxRef, MessageId, OutboxId, RemoteRef};
use rusqlite::params;

impl SqliteStore {
    /// Keep `remote` as the address of `message`, destroyed and being deleted by entry `outbox`.
    pub(super) fn keep_destroyed(
        &self,
        account: AccountId,
        message: MessageId,
        remote: &RemoteRef,
        outbox: OutboxId,
    ) -> Result<(), StoreError> {
        let (mailbox, uidvalidity, uid, uidl) = crate::remote_row::columns(remote);
        self.connection().execute(
            "INSERT INTO destroyed (account, mailbox, uidvalidity, uid, uidl, message, outbox)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (account, mailbox, COALESCE(uidvalidity, -1),
                          COALESCE(uid, -1), COALESCE(uidl, ''))
             DO UPDATE SET message = excluded.message, outbox = excluded.outbox",
            params![
                account.to_string(),
                mailbox,
                uidvalidity,
                uid,
                uidl,
                message.to_string(),
                outbox.as_i64()
            ],
        )?;
        Ok(())
    }

    /// The kept addresses in one mailbox.
    pub(super) fn destroyed_in(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT mailbox, uidvalidity, uid, uidl FROM destroyed
             WHERE account = ?1 AND mailbox = ?2",
        )?;
        let rows = stmt.query_map(
            params![mailbox.account.to_string(), mailbox.path],
            super::remote_columns,
        )?;
        rows.map(|row| super::remote_from_columns(row?)).collect()
    }

    /// Where a destroyed `message` is on the server, while its deletion has not been answered.
    pub(super) fn destroyed_of(
        &self,
        account: AccountId,
        message: MessageId,
    ) -> Result<Vec<RemoteRef>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT mailbox, uidvalidity, uid, uidl FROM destroyed
             WHERE account = ?1 AND message = ?2",
        )?;
        let rows = stmt.query_map(
            params![account.to_string(), message.to_string()],
            super::remote_columns,
        )?;
        rows.map(|row| super::remote_from_columns(row?)).collect()
    }

    /// The server no longer lists `remote`, or a move sent it where nobody said.
    pub(super) fn forget_destroyed(
        &self,
        account: AccountId,
        remote: &RemoteRef,
    ) -> Result<(), StoreError> {
        let (mailbox, uidvalidity, uid, uidl) = crate::remote_row::columns(remote);
        self.connection().execute(
            "DELETE FROM destroyed WHERE account = ?1 AND mailbox = ?2
             AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
            params![account.to_string(), mailbox, uidvalidity, uid, uidl],
        )?;
        Ok(())
    }

    /// Every kept address in a mailbox that is gone or renumbered: none of them names anything.
    pub(super) fn forget_destroyed_in(
        &self,
        account: AccountId,
        path: &str,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM destroyed WHERE account = ?1 AND mailbox = ?2",
            params![account.to_string(), path],
        )?;
        Ok(())
    }

    /// A move sent before the deletion put the message at `to`: the deletion goes there.
    pub(super) fn remap_destroyed(
        &self,
        account: AccountId,
        from: &RemoteRef,
        to: &RemoteRef,
    ) -> Result<(), StoreError> {
        let (mailbox, uidvalidity, uid, uidl) = crate::remote_row::columns(to);
        let (was_mailbox, was_uidvalidity, was_uid, was_uidl) = crate::remote_row::columns(from);
        self.connection().execute(
            "UPDATE OR REPLACE destroyed SET mailbox = ?6, uidvalidity = ?7, uid = ?8, uidl = ?9
             WHERE account = ?1 AND mailbox = ?2
             AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
            params![
                account.to_string(),
                was_mailbox,
                was_uidvalidity,
                was_uid,
                was_uidl,
                mailbox,
                uidvalidity,
                uid,
                uidl
            ],
        )?;
        Ok(())
    }

    /// The deletion queued as `outbox` was answered.
    pub(super) fn settle_destroyed(
        &self,
        outbox: OutboxId,
        answered: Answered,
    ) -> Result<(), StoreError> {
        let sql = match answered {
            Answered::Done => "UPDATE destroyed SET outbox = NULL WHERE outbox = ?1",
            Answered::Refused => "DELETE FROM destroyed WHERE outbox = ?1",
        };
        self.connection().execute(sql, params![outbox.as_i64()])?;
        Ok(())
    }
}
