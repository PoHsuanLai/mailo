//! VIPs (migration 0043). `memory/vips.rs` keeps the same rules for the in-memory store; the
//! parity tests in `tests/store/vips.rs` hold the two to each other.

use super::SqliteStore;
use crate::StoreError;
use crate::changes::Topics;
use crate::contact::learn;
use crate::contact::vip::{Vip, listed};
use rusqlite::params;

impl SqliteStore {
    pub(super) fn every_vip(&self) -> Result<Vec<Vip>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached("SELECT address, name FROM vips ORDER BY address")?;
        let rows = stmt.query_map([], |r| {
            Ok(Vip {
                address: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(listed(out))
    }

    pub(super) fn mark_vip(&self, address: &str, name: Option<&str>) -> Result<Vip, StoreError> {
        let address =
            learn::normalise(address).ok_or_else(|| StoreError::BadAddress(address.to_owned()))?;
        let name = learn::clean_name(name);
        let db = self.touch(Topics::CONTACTS | Topics::SUMMARIES);
        db.execute(
            "INSERT INTO vips (address, name) VALUES (?1, ?2)
             ON CONFLICT (address) DO UPDATE SET name = coalesce(excluded.name, vips.name)",
            params![address, name],
        )?;
        let name: Option<String> = db.query_row(
            "SELECT name FROM vips WHERE address = ?1",
            params![address],
            |r| r.get(0),
        )?;
        Ok(Vip { address, name })
    }

    pub(super) fn unmark_vip(&self, address: &str) -> Result<bool, StoreError> {
        let Some(address) = learn::normalise(address) else {
            return Ok(false);
        };
        let gone = self
            .touch(Topics::CONTACTS | Topics::SUMMARIES)
            .execute("DELETE FROM vips WHERE address = ?1", params![address])?;
        Ok(gone > 0)
    }
}
