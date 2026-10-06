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
//! The scan reads every text column except the few that hold prose and never an id (message
//! bodies and snippets, [`PROSE`]), so a large store is not read whole for it.
//!
//! The rows go in one immediate transaction, holding the store's one connection, so no `put` of
//! this process runs meanwhile. A file no remaining blob row names (two blobs with one hash are
//! one file) is set aside inside the transaction, deleted once it commits, and put back if it
//! does not, so a failed commit never leaves a row naming a file that is gone. Another process
//! storing the very same bytes at the same moment is the one case this cannot see.

use super::SqliteStore;
use crate::StoreError;
use porter_core::AccountId;
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
                    db.prepare(&format!("{} WHERE account = ?1", select(&db, &table)?))?;
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
            let mut stmt = db.prepare(&select(&db, &table)?)?;
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
        let mut aside = Vec::new();
        for path in paths {
            let shared: Option<i64> = db
                .query_row(
                    "SELECT 1 FROM blobs WHERE path = ?1 LIMIT 1",
                    [&path],
                    |r| r.get(0),
                )
                .optional()?;
            if shared.is_none() && self.blobs.set_aside(&path).is_ok() {
                aside.push(path);
            }
        }
        match tx.commit() {
            Ok(()) => {
                // A file left behind costs disk, not mail: the row that named it is gone.
                for path in &aside {
                    let _ = self.blobs.drop_aside(path);
                }
                Ok(Some(freed))
            }
            Err(error) => {
                for path in &aside {
                    let _ = self.blobs.bring_back(path);
                }
                Err(error.into())
            }
        }
    }
}

/// Columns that hold prose and never an id: what the scan does not read.
const PROSE: [&str; 3] = ["body_text", "snippet", "fts_text"];

/// Every ordinary table: not SQLite's own, not a virtual (full-text search) table, and not one of
/// a virtual table's shadow tables (`messages_fts_data` and the like), whose text is the
/// messages' again. Matched by name here, not by `LIKE`, whose `_` is a wildcard.
fn tables(db: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt =
        db.prepare("SELECT name, coalesce(sql, '') FROM sqlite_master WHERE type = 'table'")?;
    let all = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(kept_tables(&all))
}

/// Of every table as `(name, sql)`, the ones the scan reads.
fn kept_tables(all: &[(String, String)]) -> Vec<String> {
    let virtuals: Vec<&str> = all
        .iter()
        .filter(|(_, sql)| {
            sql.trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE VIRTUAL TABLE")
        })
        .map(|(name, _)| name.as_str())
        .collect();
    all.iter()
        .map(|(name, _)| name)
        .filter(|name| !name.starts_with("sqlite_"))
        .filter(|name| {
            !virtuals.iter().any(|v| {
                *name == v
                    || name
                        .strip_prefix(v)
                        .is_some_and(|rest| rest.starts_with('_'))
            })
        })
        .cloned()
        .collect()
}

/// `SELECT` of every column of `table` the scan reads.
fn select(db: &Connection, table: &str) -> Result<String, StoreError> {
    let mut stmt = db.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let columns: Vec<String> = stmt
        .query_map([table], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|column| !PROSE.contains(&column.as_str()))
        .map(|column| format!("\"{column}\""))
        .collect();
    Ok(format!("SELECT {} FROM \"{table}\"", columns.join(", ")))
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
    use super::{kept_tables, uuids};

    #[test]
    fn full_text_search_tables_are_skipped_by_name_and_drafts_is_not() {
        let all: Vec<(String, String)> = [
            ("drafts", "CREATE TABLE drafts (id TEXT)"),
            ("messages", "CREATE TABLE messages (id TEXT)"),
            (
                "messages_fts",
                "CREATE VIRTUAL TABLE messages_fts USING fts5(subject)",
            ),
            (
                "messages_fts_data",
                "CREATE TABLE 'messages_fts_data'(id INTEGER)",
            ),
            ("messages_fts_idx", "CREATE TABLE 'messages_fts_idx'(segid)"),
            ("sqlite_sequence", "CREATE TABLE sqlite_sequence(name,seq)"),
            ("softs", "CREATE TABLE softs (id TEXT)"),
        ]
        .into_iter()
        .map(|(name, sql)| (name.to_owned(), sql.to_owned()))
        .collect();
        assert_eq!(kept_tables(&all), ["drafts", "messages", "softs"]);
    }

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
