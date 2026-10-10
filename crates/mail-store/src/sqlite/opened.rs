//! History: which conversations were opened, and when they last were (migration 0029).
//! `memory/opened.rs` keeps the same rules for the in-memory store; the parity tests in
//! `tests/store/opened.rs` hold the two to each other.

use super::SqliteStore;
use super::read::SUMMARY_COLUMNS;
use super::row::from_time;
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{ThreadId, ThreadSummary};
use rusqlite::params;

impl SqliteStore {
    /// Note that `thread` was opened at `at`: its row moves to the newest if it has one and is
    /// made if not, and every row older than the retention goes. A conversation the store does
    /// not hold is not recorded, since there would be nothing to list.
    pub(super) fn write_opened(
        &self,
        thread: ThreadId,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let db = self.connection();
        db.execute(
            "INSERT INTO opened (thread, opened_at)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM threads WHERE id = ?1)
             ON CONFLICT (thread) DO UPDATE SET opened_at = excluded.opened_at WHERE excluded.opened_at > opened.opened_at",
            params![thread.to_string(), from_time(at)],
        )?;
        db.execute(
            "DELETE FROM opened WHERE opened_at < ?1",
            params![from_time(crate::opened::cutoff(at))],
        )?;
        Ok(())
    }

    /// The conversations in History, newest open first, ties by thread id.
    pub(super) fn load_opened(&self) -> Result<Vec<ThreadSummary>, StoreError> {
        let db = self.reader();
        let sql = format!(
            "SELECT {} FROM opened o JOIN thread_summary ts ON ts.thread = o.thread \
             ORDER BY o.opened_at DESC, o.thread",
            SUMMARY_COLUMNS.replace("thread,", "ts.thread,"),
        );
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(self.read_summary(row)?);
        }
        Ok(out)
    }
}
