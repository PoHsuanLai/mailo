//! Saved views. Local rows, never on a server; each is its [`View`]'s serde form whole.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::StoreError;
use mail_domain::{View, ViewId};
use rusqlite::params;

impl SqliteStore {
    pub(super) fn load_views(&self) -> Result<Vec<View>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached("SELECT view FROM views ORDER BY position, id")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(json("View", &row.get::<_, String>(0)?)?);
        }
        Ok(out)
    }

    pub(super) fn write_view(&self, view: &View) -> Result<(), StoreError> {
        // An upsert that leaves `position` alone, so editing a view does not move it.
        self.connection().execute(
            "INSERT INTO views (id, view, position)
             VALUES (?1, ?2, (SELECT coalesce(max(position), 0) + 1 FROM views))
             ON CONFLICT (id) DO UPDATE SET view = excluded.view",
            params![view.id.to_string(), to_json("View", view)?],
        )?;
        Ok(())
    }

    pub(super) fn remove_view(&self, id: ViewId) -> Result<(), StoreError> {
        let gone = self
            .connection()
            .execute("DELETE FROM views WHERE id = ?1", params![id.to_string()])?;
        if gone == 0 {
            return Err(StoreError::NoView(id));
        }
        Ok(())
    }
}
