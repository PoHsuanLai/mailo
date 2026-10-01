//! When each account was first watched, which is where "new" begins.
//!
//! An account's first sync stores every message it has ever received, a few hundred per pass and
//! over many passes, and every one of them is "stored for the first time". Announcing those would
//! raise thousands of notifications for mail the user read years ago. So the first watched pass
//! writes down when it ran, once, and a message dated before that is backfill however late the
//! pass that stores it.
//!
//! Persisted (`notify_floor`, migration 0010) and never moved, so a restart neither re-arms —
//! which would silence the mail that arrived while nothing was watching — nor forgets. A message
//! is never announced twice either way: it is an arrival only on the pass that first stores it,
//! and a stored message is never stored for the first time again.

use chrono::{DateTime, Utc};
use mail_domain::AccountId;
use mail_store::SqliteStore;

/// The account's floor, arming it at `now` if nothing has watched it before.
pub fn armed(
    store: &SqliteStore,
    account: AccountId,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let db = store.connection();
    // `OR IGNORE`: the first writer wins and every later pass reads what it wrote.
    db.execute(
        "INSERT OR IGNORE INTO notify_floor (account, armed_at) VALUES (?1, ?2)",
        rusqlite::params![account.to_string(), now.to_rfc3339()],
    )
    .map_err(|e| format!("cannot arm notifications: {e}"))?;
    let stored: String = db
        .query_row(
            "SELECT armed_at FROM notify_floor WHERE account = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .map_err(|e| format!("cannot read when notifications were armed: {e}"))?;
    DateTime::parse_from_rfc3339(&stored)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|e| format!("the stored notification floor {stored:?} is unreadable: {e}"))
}
