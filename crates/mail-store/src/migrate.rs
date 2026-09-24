//! Forward-only schema migrations.
//!
//! `accounts` holds a serialized `AccountPlan`, so a field added to that type breaks startup
//! for every existing row unless it carries `#[serde(default)]` and a migration exists. Both
//! are required by `CONVENTIONS.md` section 3.

use crate::StoreError;
use rusqlite::Connection;

/// Every migration, in order. Index 0 produces schema version 1.
///
/// **Append only.** A released migration is a fact about databases that already exist on
/// disk; editing one does not change them, it only makes this build disagree with them.
///
/// Public so `tests/upgrade.rs` can build a database at any prior version by applying a prefix
/// of it. The alternative is a checked-in binary fixture per version, which drifts from the
/// migration it is supposed to represent the moment anyone edits one.
pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (
        2,
        include_str!("../migrations/0002_remote_map_identity.sql"),
    ),
    (3, include_str!("../migrations/0003_list_order_index.sql")),
    (4, include_str!("../migrations/0004_fts_segmented.sql")),
    (5, include_str!("../migrations/0005_fts_recipients.sql")),
    (6, include_str!("../migrations/0006_fts_one_tokenizer.sql")),
    (7, include_str!("../migrations/0007_resummarize.sql")),
    (8, include_str!("../migrations/0008_remap_imap.sql")),
    (9, include_str!("../migrations/0009_folders.sql")),
    (10, include_str!("../migrations/0010_notify_floor.sql")),
    (11, include_str!("../migrations/0011_receipts.sql")),
    (
        12,
        include_str!("../migrations/0012_reparse_raw_headers.sql"),
    ),
    (13, include_str!("../migrations/0013_contacts.sql")),
    (14, include_str!("../migrations/0014_templates.sql")),
    (
        15,
        include_str!("../migrations/0015_unmix_body_batches.sql"),
    ),
    (16, include_str!("../migrations/0016_invite_answers.sql")),
];

/// The schema version this build expects.
pub const EXPECTED_VERSION: u32 = 16;

/// Bring `db` up to [`EXPECTED_VERSION`], creating it if it is empty.
///
/// Refuses to touch a database written by a newer build: migrating backwards would mean
/// guessing what a future version meant, and guessing wrong silently corrupts the user's mail.
pub fn migrate(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| StoreError::Db(e.to_string()))?;

    let current = current_version(db)?;
    if current > EXPECTED_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: current,
            expected: EXPECTED_VERSION,
        });
    }

    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        // One transaction per migration: a failure half way leaves the database at the last
        // version that fully applied, rather than in a shape no version describes.
        let tx = db
            .unchecked_transaction()
            .map_err(|e| StoreError::Db(e.to_string()))?;
        tx.execute_batch(sql)
            .map_err(|e| StoreError::Db(format!("migration {version}: {e}")))?;
        // 0001 seeds its own row; later migrations must record themselves.
        if *version > 1 {
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
                [version],
            )
            .map_err(|e| StoreError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| StoreError::Db(e.to_string()))?;
    }
    Ok(())
}

/// The highest version applied, or 0 for an empty database.
fn current_version(db: &Connection) -> Result<u32, StoreError> {
    let exists: bool = db
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='schema_version'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| StoreError::Db(e.to_string()))?;
    if !exists {
        return Ok(0);
    }
    db.query_row(
        "SELECT coalesce(max(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )
    .map_err(|e| StoreError::Db(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrating_an_empty_database_reaches_the_expected_version() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        assert_eq!(current_version(&db).unwrap(), EXPECTED_VERSION);
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        // Every startup calls this. It must not re-run 0001 and fail on "table exists".
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        migrate(&db).expect("second migrate must be a no-op");
        assert_eq!(current_version(&db).unwrap(), EXPECTED_VERSION);
    }

    #[test]
    fn a_newer_database_is_refused_rather_than_downgraded() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (99, datetime('now'))",
            [],
        )
        .unwrap();
        match migrate(&db) {
            Err(StoreError::SchemaTooNew { found, expected }) => {
                assert_eq!((found, expected), (99, EXPECTED_VERSION));
            }
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        // Declared in the schema, but SQLite ignores the declaration unless the pragma is on
        // per connection. Without this, a cascade delete silently leaves orphans.
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        let on: bool = db
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(on, "foreign_keys must be on for cascades to work");
    }
}
