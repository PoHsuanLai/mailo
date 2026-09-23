//! Messages queued to be re-read from their stored bytes (migration 0012).
//!
//! On `SqliteStore` alone, not on [`crate::Store`]: the queue exists only because databases on
//! disk were written by an older parser, and a `MemoryStore` never holds anything an older
//! build wrote. Draining it needs `mail-mime`, which this crate does not see, so the runtime
//! reads the queue, re-parses, writes each message back as an ordinary upsert, and says so here.

use super::SqliteStore;
use super::row::uuid;
use crate::StoreError;
use mail_domain::MessageId;
use rusqlite::params;

impl SqliteStore {
    /// Every message waiting to be re-parsed.
    pub fn reparse_queue(&self) -> Result<Vec<MessageId>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare("SELECT message FROM messages_to_reparse ORDER BY message")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(MessageId::from_uuid(uuid(
                "messages_to_reparse.message",
                &row?,
            )?));
        }
        Ok(out)
    }

    /// Take `message` off the queue: rewritten, or found not worth another try.
    pub fn reparsed(&self, message: MessageId) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM messages_to_reparse WHERE message = ?1",
            params![message.to_string()],
        )?;
        Ok(())
    }
}
