//! Removing an account from the database: its row, everything that names it, and the stored bytes
//! that nothing else still uses.
//!
//! Every table that names an account follows its row by `ON DELETE CASCADE` (contacts by
//! `SET NULL`). Blobs do not: they are shared by content, so one attachment two accounts received
//! is one blob, and their ids are written into JSON (attachments, drafts, queued sends, undo
//! patches) as well as into `messages.body_raw`. So the blobs considered are only those whose id
//! appears in a row that named the account, and one is deleted only when, with the account gone,
//! its id appears in no text anywhere else in the database. Reading every text column for the id
//! rather than listing the columns that hold one means a reference added later is not missed:
//! the cost of a mistake is a blob kept, never a message that has lost its bytes.
//!
//! The rows go in one immediate transaction, which another writer waits behind; the files go
//! after it commits, and only a file no remaining blob row names (two blobs with one hash are one
//! file).

use super::SqliteStore;
use crate::StoreError;
use mail_domain::AccountId;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

/// What removing an account freed besides its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Freed {
    /// Blobs deleted: raw messages and attachment parts.
    pub blobs: u64,
    /// Their size, in bytes.
    pub bytes: u64,
}

impl SqliteStore {
    /// Delete `account` and everything that names it, and the blobs only it used. `None` when no
    /// account has that id, and then nothing changed.
    pub fn remove_account(&self, account: AccountId) -> Result<Option<Freed>, StoreError> {
        let db = self.connection();
        let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)?;
        let id = account.to_string();
        let mut candidates = BTreeSet::new();
        for table in tables(&db)? {
            if has_account_column(&db, &table)? {
                let mut stmt =
                    db.prepare(&format!("SELECT * FROM \"{table}\" WHERE account = ?1"))?;
                each_text(&mut stmt, params![id], |text| {
                    candidates.extend(uuids(text));
                    Scan::Go
                })?;
            }
        }
        candidates.retain(|blob| is_blob(&db, blob).unwrap_or(false));
        if db.execute("DELETE FROM accounts WHERE id = ?1", [&id])? == 0 {
            return Ok(None);
        }
        // What still names a candidate keeps it.
        for table in tables(&db)? {
            if candidates.is_empty() {
                break;
            }
            if table == "blobs" {
                continue;
            }
            let mut stmt = db.prepare(&format!("SELECT * FROM \"{table}\""))?;
            each_text(&mut stmt, [], |text| {
                for named in uuids(text) {
                    candidates.remove(&named);
                }
                if candidates.is_empty() {
                    Scan::Stop
                } else {
                    Scan::Go
                }
            })?;
        }
        let mut freed = Freed::default();
        let mut paths = Vec::new();
        for blob in &candidates {
            let (size, path): (i64, Option<String>) =
                db.query_row("SELECT size, path FROM blobs WHERE id = ?1", [blob], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            db.execute("DELETE FROM blobs WHERE id = ?1", [blob])?;
            freed.blobs += 1;
            freed.bytes += u64::try_from(size).unwrap_or(0);
            paths.extend(path);
        }
        tx.commit()?;
        for path in paths {
            let shared: Option<i64> = db
                .query_row(
                    "SELECT 1 FROM blobs WHERE path = ?1 LIMIT 1",
                    [&path],
                    |r| r.get(0),
                )
                .optional()?;
            if shared.is_none() {
                // A file left behind costs disk, not mail: the row that named it is gone.
                let _ = self.blobs.forget_file(&path);
            }
        }
        Ok(Some(freed))
    }
}

/// Every ordinary table: not SQLite's own, and not full-text search's, whose text is the
/// messages' again.
fn tables(db: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt = db.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
           AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' AND name NOT LIKE '%_fts%'",
    )?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

fn has_account_column(db: &Connection, table: &str) -> Result<bool, StoreError> {
    let found: i64 = db.query_row(
        "SELECT count(*) FROM pragma_table_info(?1) WHERE name = 'account'",
        [table],
        |r| r.get(0),
    )?;
    Ok(found > 0)
}

fn is_blob(db: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(db
        .query_row("SELECT 1 FROM blobs WHERE id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
        .is_some())
}

/// Whether a scan goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    Go,
    Stop,
}

/// Hand every text value of every row `stmt` returns to `seen`, until it says stop.
fn each_text(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
    mut seen: impl FnMut(&str) -> Scan,
) -> Result<(), StoreError> {
    let columns = stmt.column_count();
    let mut rows = stmt.query(params)?;
    while let Some(row) = rows.next()? {
        for at in 0..columns {
            if let ValueRef::Text(bytes) = row.get_ref(at)?
                && let Ok(text) = std::str::from_utf8(bytes)
                && seen(text) == Scan::Stop
            {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Every lowercase hyphenated UUID in `text`, the way ids are stored.
fn uuids(text: &str) -> impl Iterator<Item = String> + '_ {
    const LEN: usize = 36;
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(LEN - 1)).filter_map(move |start| {
        let window = &bytes[start..start + LEN];
        let shaped = window.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        });
        // Not the tail of a longer run of the same characters.
        let bounded =
            |at: Option<&u8>| at.is_none_or(|byte| !byte.is_ascii_hexdigit() && *byte != b'-');
        (shaped
            && bounded(start.checked_sub(1).and_then(|before| bytes.get(before)))
            && bounded(bytes.get(start + LEN)))
        .then(|| String::from_utf8_lossy(window).into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::uuids;

    #[test]
    fn ids_are_found_wherever_they_are_written() {
        const ID: &str = "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d";
        // (text, the ids found)
        let cases: &[(String, Vec<&str>)] = &[
            (ID.to_owned(), vec![ID]),
            (format!(r#"[{{"name":"a.pdf","blob":"{ID}"}}]"#), vec![ID]),
            (format!("{ID},{ID}"), vec![ID, ID]),
            // Too long on either side is some other token.
            (format!("a{ID}"), vec![]),
            (format!("{ID}0"), vec![]),
            (ID.to_uppercase(), vec![]),
            ("no ids here".to_owned(), vec![]),
        ];
        for (text, expect) in cases {
            assert_eq!(uuids(text).collect::<Vec<_>>(), *expect, "{text}");
        }
    }
}
