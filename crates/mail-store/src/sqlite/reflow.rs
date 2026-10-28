//! Plain drafts and templates queued to have their text written as flowed text (migration 0038).
//!
//! On `SqliteStore` alone, as the reparse queue is: only a database an older build wrote holds
//! such a text. The rewrite is the editor's, in `mail-core`, which this crate does not see, so
//! the caller reads the queue, rewrites each text and hands it back here.

use super::SqliteStore;
use super::row::uuid;
use crate::StoreError;
use mail_domain::{DraftId, TemplateId};
use rusqlite::params;

/// What a queued text belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    Draft(DraftId),
    Template(TemplateId),
}

impl Kept {
    /// The table this lives in, the queue that names it and its column there, and its id.
    fn tables(self) -> (&'static str, &'static str, &'static str, String) {
        match self {
            Kept::Draft(id) => ("drafts", "drafts_to_reflow", "draft", id.to_string()),
            Kept::Template(id) => (
                "templates",
                "templates_to_reflow",
                "template",
                id.to_string(),
            ),
        }
    }
}

impl SqliteStore {
    /// Every draft and template whose text waits to be written as flowed text, with that text.
    pub fn reflow_queue(&self) -> Result<Vec<(Kept, String)>, StoreError> {
        let db = self.reader();
        let mut out = Vec::new();
        let mut stmt = db.prepare(
            "SELECT q.draft, d.body_text FROM drafts_to_reflow q
             JOIN drafts d ON d.id = q.draft ORDER BY q.draft",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, text) = row?;
            let id = DraftId::from_uuid(uuid("drafts_to_reflow.draft", &id)?);
            out.push((Kept::Draft(id), text));
        }
        let mut stmt = db.prepare(
            "SELECT q.template, t.body_text FROM templates_to_reflow q
             JOIN templates t ON t.id = q.template ORDER BY q.template",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, text) = row?;
            let id = TemplateId::from_uuid(uuid("templates_to_reflow.template", &id)?);
            out.push((Kept::Template(id), text));
        }
        Ok(out)
    }

    /// Keep `text` as `kept`'s text and take it off the queue, together. Only the text is
    /// written: nothing else about it changes, it is not recorded as an edit, and nothing goes
    /// to the server.
    pub fn reflowed(&self, kept: Kept, text: &str) -> Result<(), StoreError> {
        let (table, queue, column, id) = kept.tables();
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            &format!("UPDATE {table} SET body_text = ?2 WHERE id = ?1"),
            params![id, text],
        )?;
        tx.execute(
            &format!("DELETE FROM {queue} WHERE {column} = ?1"),
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }
}
