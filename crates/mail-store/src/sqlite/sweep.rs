//! Taking back the stored bytes nothing refers to any more.
//!
//! Nothing releases a blob when a message is deleted, when a body fetch replaces a headers-only
//! raw, or when a fetch stores the headers it only needed to read, so unreferenced rows and files
//! pile up. A sweep finds them the way removing an account does ([`super::forget`]): by reading
//! every text column of every table for the ids of blobs, so a reference added to the schema later
//! is not missed, and the cost of a mistake is a blob kept, never a message that lost its bytes.
//!
//! What is taken is only what has gone unused for [`BLOB_GRACE`]. A blob is stored before the row
//! that names it is written, so a fresh one is ahead of its reference, not unreferenced; and a
//! put of bytes already kept says they are in use again ([`crate::blob`]), so one a message is
//! about to name is not taken between the scan and the delete.
//!
//! The rows go in one short transaction and their files are set aside inside it, as removing an
//! account does, and deleted after it commits, when the writer is free again.

use super::SqliteStore;
use super::forget::{Scan, each_text, select, tables, uuids};
use crate::StoreError;
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, SystemTime};

/// How long a blob or a file with nothing naming it is left alone.
const BLOB_GRACE: Duration = Duration::from_secs(60 * 60);

/// How often [`SqliteStore::sweep_blobs_daily`] sweeps.
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The marker, in the blob root, whose time is when a sweep last ran. In a file beside the blobs
/// rather than in the database so it is one answer for every process using them and needs no
/// migration.
const MARKER: &str = ".swept";

/// What a sweep took back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Swept {
    /// Blob rows deleted.
    pub blobs: u64,
    /// Their size, in bytes.
    pub bytes: u64,
    /// Files deleted: those of the rows, and those no row named.
    pub files: u64,
    /// Files that should have been deleted and could not be: left for the next sweep.
    pub left: u64,
}

impl SqliteStore {
    /// Sweep the blobs at most once a day, whichever process asks: `None` when one ran within the
    /// day before `now`. Takes the time a sweep ran from a marker beside the blobs.
    pub fn sweep_blobs_daily(&self, now: DateTime<Utc>) -> Result<Option<Swept>, StoreError> {
        let marker = self.files().root().join(MARKER);
        let at = SystemTime::from(now);
        let recent = std::fs::metadata(&marker)
            .and_then(|meta| meta.modified())
            // A marker from the future (a clock set back) is not older than a day either.
            .is_ok_and(|ran| at.duration_since(ran).is_ok_and(|age| age < EVERY) || ran > at);
        if recent {
            return Ok(None);
        }
        let swept = self.sweep_blobs(now)?;
        std::fs::create_dir_all(self.files().root())
            .and_then(|()| std::fs::write(&marker, now.to_rfc3339()))
            .map_err(|e| StoreError::Blob(MARKER.to_owned(), e.to_string()))?;
        Ok(Some(swept))
    }

    /// Delete the blobs nothing names that have gone unused for an hour before `now`, and the
    /// files no blob has: the files of what was deleted, an orphan an hour old, and a half-written
    /// file left by a crash.
    ///
    /// Reads under no lock but a reader's, and holds the writer only to delete rows and move their
    /// files aside; the files are deleted after, with it let go of.
    pub fn sweep_blobs(&self, now: DateTime<Utc>) -> Result<Swept, StoreError> {
        // As SQLite writes `datetime('now')`, which is what a row's time is, so the two compare as
        // text.
        let cutoff: String = self.reader().query_row(
            "SELECT datetime(?1, 'unixepoch')",
            [(now - BLOB_GRACE).timestamp()],
            |r| r.get(0),
        )?;
        let mut swept = Swept::default();

        // Unused for a while, and then not named by anything.
        let mut unused: BTreeMap<String, (i64, Option<String>)> = BTreeMap::new();
        {
            let db = self.reader();
            let mut stmt = db.prepare("SELECT id, size, path FROM blobs WHERE created_at < ?1")?;
            let rows = stmt.query_map(params![cutoff], |r| {
                Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?)))
            })?;
            for row in rows {
                let (id, kept) = row?;
                unused.insert(id, kept);
            }
            for table in tables(&db)? {
                if unused.is_empty() {
                    break;
                }
                if table == "blobs" {
                    continue;
                }
                let columns = select(&db, &table)?;
                let mut stmt = db.prepare(&columns)?;
                each_text(&mut stmt, [], |text| {
                    for named in uuids(text) {
                        unused.remove(&named);
                    }
                    if unused.is_empty() {
                        Scan::Stop
                    } else {
                        Scan::Go
                    }
                })?;
            }
        }

        let mut aside = Vec::new();
        if !unused.is_empty() {
            let db = self.connection();
            let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)?;
            for (id, (size, path)) in &unused {
                // Looked at again with the writer held: a put since the scan has made it fresh.
                let gone = db.execute(
                    "DELETE FROM blobs WHERE id = ?1 AND created_at < ?2",
                    params![id, cutoff],
                )?;
                if gone == 0 {
                    continue;
                }
                swept.blobs += 1;
                swept.bytes += u64::try_from(*size).unwrap_or(0);
                let Some(path) = path else { continue };
                let shared: Option<i64> = db
                    .query_row("SELECT 1 FROM blobs WHERE path = ?1 LIMIT 1", [path], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if shared.is_none() {
                    match self.files().set_aside(path) {
                        Ok(()) => aside.push(path.clone()),
                        // Left where it is; with no row, a later sweep finds it as an orphan.
                        Err(_) => swept.left += 1,
                    }
                }
            }
            if let Err(error) = tx.commit() {
                for path in &aside {
                    // Best-effort: the commit's own error is what is reported.
                    let _ = self.files().bring_back(path);
                }
                return Err(error.into());
            }
        }
        for path in &aside {
            match self.files().drop_aside(path) {
                Ok(()) => swept.files += 1,
                Err(_) => swept.left += 1,
            }
        }

        // Files no row names.
        let named: HashSet<String> = {
            let db = self.reader();
            let mut stmt = db.prepare("SELECT path FROM blobs WHERE path IS NOT NULL")?;
            let paths = stmt.query_map([], |r| r.get::<_, String>(0))?;
            paths.collect::<Result<_, _>>()?
        };
        let old_enough = SystemTime::from(now)
            .checked_sub(BLOB_GRACE)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        for (rel, modified) in self.files().stored_files()? {
            if modified > old_enough {
                continue;
            }
            let removed = if rel.ends_with(".part") {
                // A write that never finished: the file it was to become is not there to lose.
                self.files().remove_stored(&rel)
            } else if let Some(file) = rel.strip_suffix(".aside") {
                if named.contains(file) {
                    // Set aside by a sweep that died before it committed: its row is still here.
                    self.files().bring_back(file)
                } else {
                    self.files().remove_stored(&rel)
                }
            } else if named.contains(&rel) {
                continue;
            } else {
                self.files().remove_stored(&rel)
            };
            match removed {
                Ok(()) => swept.files += 1,
                Err(_) => swept.left += 1,
            }
        }
        Ok(swept)
    }
}
