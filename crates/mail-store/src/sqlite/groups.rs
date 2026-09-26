//! Contact groups in SQLite. Kept in step with `memory/groups.rs`, which the parity tests in
//! `tests/groups.rs` hold it to.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::StoreError;
use crate::contact::{Group, GroupId};
use rusqlite::{OptionalExtension, params};

const COLUMNS: &str = "id, uid, name, members, home";

type Columns = (String, Option<String>, String, String, String);

fn columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<Columns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn group((id, uid, name, members, home): Columns) -> Result<Group, StoreError> {
    Ok(Group {
        id: GroupId(id),
        uid,
        name,
        members: json("Group.members", &members)?,
        home: json("Group.home", &home)?,
    })
}

impl SqliteStore {
    pub(super) fn every_group(&self) -> Result<Vec<Group>, StoreError> {
        // `NOCASE` folds ASCII only, which is what the in-memory store's ordering does too.
        let sql = format!("SELECT {COLUMNS} FROM contact_groups ORDER BY name COLLATE NOCASE, id");
        let db = self.reader();
        let mut stmt = db.prepare_cached(&sql)?;
        let rows = stmt.query_map([], columns)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(group(row?)?);
        }
        Ok(out)
    }

    pub(super) fn one_group(&self, id: &GroupId) -> Result<Option<Group>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM contact_groups WHERE id = ?1");
        let row = self
            .reader()
            .query_row(&sql, params![id.0], columns)
            .optional()?;
        row.map(group).transpose()
    }

    pub(super) fn write_group(&self, group: &Group) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT OR REPLACE INTO contact_groups (id, uid, name, members, home)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group.id.0,
                group.uid,
                group.name,
                to_json("Group.members", &group.members)?,
                to_json("Group.home", &group.home)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn drop_group(&self, id: &GroupId) -> Result<bool, StoreError> {
        let gone = self
            .connection()
            .execute("DELETE FROM contact_groups WHERE id = ?1", params![id.0])?;
        Ok(gone > 0)
    }
}
