//! The server's copies of mail a POP3 account holds here, for a pass that removes them
//! (`mail_core::offline::Remove`).
//!
//! Which copies may go is the store's to answer, from what it holds: a message whose body is here
//! in full, old enough, or deleted here. Removing one is two steps, and this is the first: the
//! address is let go of ([`SqliteStore::let_go_of`]) before the server is asked, so the message
//! stays here, held at no address, and the sync that next lists the maildrop does not read a
//! message missing from it as one deleted elsewhere, which would delete it here too. Asked and
//! refused, the address comes back with the next sync, which finds the message by its identity.

use super::SqliteStore;
use super::row::{from_time, to_json};
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{MailboxRef, MailboxRole, RemoteRef};
use porter_core::AccountId;
use rusqlite::params;

impl SqliteStore {
    /// The addresses in `mailbox` of messages held here in full and both dated and stored before
    /// `before`, oldest first, at most `limit`. A message's age is the later of the date it
    /// carries and the time it was stored, so a late or forged Date does not make it old.
    pub fn copies_held_before(
        &self,
        mailbox: &MailboxRef,
        before: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<RemoteRef>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached(
            "SELECT r.mailbox, r.uidvalidity, r.uid, r.uidl
             FROM remote_map r
             JOIN messages m ON m.id = r.message
             WHERE r.account = ?1 AND r.mailbox = ?2 AND m.body_raw IS NOT NULL AND m.date < ?3
               AND coalesce(m.stored_at, m.date) < ?3
             ORDER BY m.date, m.id
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![
                mailbox.account.to_string(),
                mailbox.path,
                from_time(before),
                i64::from(limit)
            ],
            super::remote_columns,
        )?;
        rows.map(|row| super::remote_from_columns(row?)).collect()
    }

    /// The addresses in `mailbox` of messages deleted here: in the Trash, or deleted forever
    /// while the server still lists them (migration 0025). At most `limit`. A message in the
    /// Trash that was never downloaded is not among them: the server's copy is the only one.
    pub fn copies_deleted_here(
        &self,
        mailbox: &MailboxRef,
        limit: u32,
    ) -> Result<Vec<RemoteRef>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached(
            "SELECT r.mailbox, r.uidvalidity, r.uid, r.uidl
             FROM remote_map r
             JOIN messages m ON m.id = r.message
             WHERE r.account = ?1 AND r.mailbox = ?2 AND m.mailbox = ?3
               AND m.body_raw IS NOT NULL
             UNION
             SELECT mailbox, uidvalidity, uid, uidl FROM destroyed
             WHERE account = ?1 AND mailbox = ?2
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![
                mailbox.account.to_string(),
                mailbox.path,
                to_json("MailboxRole", &MailboxRole::Trash)?,
                i64::from(limit)
            ],
            super::remote_columns,
        )?;
        rows.map(|row| super::remote_from_columns(row?)).collect()
    }

    /// Let go of `remotes`, the server's copies of messages about to be removed from it: each
    /// message stays, held here alone, as mail the server never had. An address of a message
    /// deleted forever is kept as it is (migration 0025's rows), so the message
    /// is not fetched again while the server still lists it.
    pub fn let_go_of(&self, account: AccountId, remotes: &[RemoteRef]) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        for remote in remotes {
            let (mailbox, uidvalidity, uid, uidl) = crate::remote_row::columns(remote);
            db.prepare_cached(
                "DELETE FROM remote_map WHERE account = ?1 AND mailbox = ?2
                 AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
            )?
            .execute(params![
                account.to_string(),
                mailbox,
                uidvalidity,
                uid,
                uidl
            ])?;
        }
        tx.commit()?;
        Ok(())
    }
}
