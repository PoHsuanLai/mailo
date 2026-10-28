//! Saved views. Local rows, never on a server; each is its [`View`]'s serde form whole.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::StoreError;
use crate::changes::Topics;
use mail_domain::{View, ViewId};
use rusqlite::params;

impl SqliteStore {
    pub(super) fn load_views(&self) -> Result<Vec<View>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached("SELECT id, view FROM views ORDER BY position, id")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            // A view this build cannot read (one a newer build wrote, with a property or a
            // clause it does not know) is left out, and left in its row, rather than failing
            // the whole list: the views it can read are still the person's. A build that knows
            // it reads it again.
            match json("View", &row.get::<_, String>(1)?) {
                Ok(view) => out.push(view),
                Err(e) => log::warn!(
                    "the saved view {} could not be read and is left out: {e}",
                    row.get::<_, String>(0)?
                ),
            }
        }
        Ok(out)
    }

    pub(super) fn write_view(&self, view: &View) -> Result<(), StoreError> {
        // An upsert that leaves `position` alone, so editing a view does not move it.
        self.touch(Topics::VIEWS).execute(
            "INSERT INTO views (id, view, position)
             VALUES (?1, ?2, (SELECT coalesce(max(position), 0) + 1 FROM views))
             ON CONFLICT (id) DO UPDATE SET view = excluded.view",
            params![view.id.to_string(), to_json("View", view)?],
        )?;
        Ok(())
    }

    pub(super) fn remove_view(&self, id: ViewId) -> Result<(), StoreError> {
        let gone = self
            .touch(Topics::VIEWS)
            .execute("DELETE FROM views WHERE id = ?1", params![id.to_string()])?;
        if gone == 0 {
            return Err(StoreError::NoView(id));
        }
        Ok(())
    }
}
