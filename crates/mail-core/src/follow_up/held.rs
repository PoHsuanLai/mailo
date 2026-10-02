//! A composer's reminder, held in `follow_up_held` (migration 0027) until its message has left.

use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Draft, DraftId, SendState, ThreadId};
use mail_store::{SqliteStore, Store};

/// A composer's reminder, waiting for its message to leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub draft: DraftId,
    pub account: AccountId,
    /// The `Message-ID` the message goes with, normalized.
    pub message_id: String,
    /// The conversation a reply answers.
    pub thread: Option<ThreadId>,
    pub at: DateTime<Utc>,
    /// When the message leaves.
    pub set: DateTime<Utc>,
}

/// Hold a reminder for `draft`, which has just been queued as `raw` to leave at `set`, replacing
/// whatever it held before. A draft that asked for none lets any earlier one go.
pub fn after_queue(
    store: &SqliteStore,
    draft: &Draft,
    raw: &[u8],
    at: Option<DateTime<Utc>>,
    set: DateTime<Utc>,
) -> Result<(), String> {
    release(store, draft.id)?;
    let Some(at) = at else {
        return Ok(());
    };
    let parsed = mail_mime::parse(raw).map_err(|e| e.to_string())?;
    let message_id = parsed
        .rfc_message_id
        .ok_or_else(|| "the message has no Message-ID to be found by".to_owned())?;
    let thread = draft
        .in_reply_to
        .and_then(|parent| store.message(parent).ok())
        .map(|parent| parent.thread);
    hold(
        store,
        &Held {
            draft: draft.id,
            account: draft.account,
            message_id,
            thread,
            at,
            set,
        },
    )
}

/// Keep `held` until its message has left.
pub fn hold(store: &SqliteStore, held: &Held) -> Result<(), String> {
    store
        .connection()
        .execute(
            "INSERT INTO follow_up_held (draft, account, message_id, thread, due_at, set_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(draft) DO UPDATE SET account = excluded.account,
                 message_id = excluded.message_id, thread = excluded.thread,
                 due_at = excluded.due_at, set_at = excluded.set_at",
            rusqlite::params![
                held.draft.to_string(),
                held.account.to_string(),
                held.message_id,
                held.thread.map(|thread| thread.to_string()),
                held.at.to_rfc3339(),
                held.set.to_rfc3339(),
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("cannot keep the reminder: {e}"))
}

/// Let a draft's held reminder go: its send was taken back, or sent again without one.
pub fn release(store: &SqliteStore, draft: DraftId) -> Result<(), String> {
    store
        .connection()
        .execute(
            "DELETE FROM follow_up_held WHERE draft = ?1",
            [draft.to_string()],
        )
        .map(|_| ())
        .map_err(|e| format!("cannot let the reminder go: {e}"))
}

/// Every held reminder. A row that does not read back is skipped: it is a reminder, not mail.
pub fn held(store: &SqliteStore) -> Vec<Held> {
    let db = store.connection();
    let Ok(mut stmt) =
        db.prepare("SELECT draft, account, message_id, thread, due_at, set_at FROM follow_up_held")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    }) else {
        return Vec::new();
    };
    let instant = |text: &str| {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.with_timezone(&Utc))
    };
    rows.filter_map(Result::ok)
        .filter_map(|(draft, account, message_id, thread, at, set)| {
            Some(Held {
                draft: DraftId::from_uuid(draft.parse().ok()?),
                account: AccountId::from_uuid(account.parse().ok()?),
                message_id,
                thread: match thread {
                    Some(thread) => Some(ThreadId::from_uuid(thread.parse().ok()?)),
                    None => None,
                },
                at: instant(&at)?,
                set: instant(&set)?,
            })
        })
        .collect()
}

/// The conversation a held reminder now belongs on, once its message has left: the one its sent
/// copy landed in, or for a reply, the one it answers once the outbox has sent it.
pub(super) fn landed(store: &SqliteStore, held: &Held) -> Option<ThreadId> {
    let found: Option<String> = store
        .connection()
        .query_row(
            "SELECT thread FROM messages WHERE account = ?1 AND rfc_message_id = ?2 LIMIT 1",
            [held.account.to_string(), held.message_id.clone()],
            |row| row.get(0),
        )
        .ok();
    if let Some(thread) = found.and_then(|id| id.parse().ok()) {
        return Some(ThreadId::from_uuid(thread));
    }
    let sent = matches!(
        store.draft(held.draft).map(|draft| draft.state),
        Ok(SendState::Sent { .. })
    );
    held.thread.filter(|_| sent)
}
