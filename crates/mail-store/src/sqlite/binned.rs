//! What is in Trash and Spam, and since when (migration 0041).

use super::SqliteStore;
use super::row::{from_time, time, to_json, uuid};
use crate::StoreError;
use crate::binned::{Binned, Settled, Soon};
use chrono::{DateTime, Utc};
use mail_domain::{MailboxRole, MessageId, ThreadId};
use rusqlite::params;

impl SqliteStore {
    /// Every message in Trash or Spam, with since when it has been there, after bringing that up
    /// to `now`: a message there that had no row is there since `now`, and a row whose message
    /// has left goes, so a message that comes back starts its time again.
    pub fn binned(&self, now: DateTime<Utc>) -> Result<Vec<Binned>, StoreError> {
        let trash = to_json("MailboxRole", &MailboxRole::Trash)?;
        let spam = to_json("MailboxRole", &MailboxRole::Spam)?;
        {
            let db = self.connection();
            let tx = db.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM binned WHERE message NOT IN
                 (SELECT id FROM messages WHERE mailbox IN (?1, ?2))",
                params![trash, spam],
            )?;
            tx.execute(
                "INSERT INTO binned (message, since, soon)
                 SELECT id, ?3, 0 FROM messages
                 WHERE mailbox IN (?1, ?2) AND id NOT IN (SELECT message FROM binned)",
                params![trash, spam, from_time(now)],
            )?;
            tx.commit()?;
        }
        let db = self.reader();
        let mut stmt = db.prepare_cached(
            "SELECT b.message, m.thread, m.account, m.mailbox, b.since, b.soon,
                    EXISTS (SELECT 1 FROM pending_changes p WHERE p.message = b.message),
                    EXISTS (SELECT 1 FROM remote_map r WHERE r.message = b.message)
             FROM binned b JOIN messages m ON m.id = b.message
             ORDER BY b.since, b.message",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mailbox: String = row.get(3)?;
            let pending: bool = row.get(6)?;
            let placed: bool = row.get(7)?;
            out.push(Binned {
                message: MessageId::from_uuid(uuid("MessageId", &row.get::<_, String>(0)?)?),
                thread: ThreadId::from_uuid(uuid("ThreadId", &row.get::<_, String>(1)?)?),
                account: mail_domain::id::account_id_from_uuid(uuid(
                    "AccountId",
                    &row.get::<_, String>(2)?,
                )?),
                mailbox: super::row::json("MailboxRole", &mailbox)?,
                since: time("binned.since", &row.get::<_, String>(4)?)?,
                soon: if row.get::<_, i64>(5)? == 1 {
                    Soon::Asked
                } else {
                    Soon::Waits
                },
                settled: match (pending, placed) {
                    (true, _) => Settled::Pending,
                    (false, true) => Settled::Yes,
                    (false, false) => Settled::Unplaced,
                },
            });
        }
        Ok(out)
    }

    /// `messages`, just moved to Trash by Delete under "Delete deletes it now", are to be erased
    /// as soon as the server holds them there. A message not in Trash here is left alone.
    pub fn erase_soon(&self, messages: &[MessageId], now: DateTime<Utc>) -> Result<(), StoreError> {
        let trash = to_json("MailboxRole", &MailboxRole::Trash)?;
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        for message in messages {
            tx.execute(
                "INSERT INTO binned (message, since, soon)
                 SELECT id, ?2, 1 FROM messages WHERE id = ?1 AND mailbox = ?3
                 ON CONFLICT (message) DO UPDATE SET soon = 1",
                params![message.to_string(), from_time(now), trash],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
