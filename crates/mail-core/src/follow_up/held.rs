//! A composer's reminder, held in `follow_up_held` (migration 0027) until its message has left.

use chrono::{DateTime, Utc};
use mail_domain::{Draft, DraftId, SendState, ThreadId};
use mail_store::{SqliteStore, Store};

/// A composer's reminder, waiting for its message to leave.
pub use mail_store::FollowUpHold as Held;

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
            account: draft.account.clone(),
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
        .hold_follow_up(held)
        .map_err(|e| format!("cannot keep the reminder: {e}"))
}

/// Let a draft's held reminder go: its send was taken back, or sent again without one.
pub fn release(store: &SqliteStore, draft: DraftId) -> Result<(), String> {
    store
        .release_follow_up(draft)
        .map_err(|e| format!("cannot let the reminder go: {e}"))
}

/// Every held reminder. A row that does not read back is skipped: it is a reminder, not mail.
pub fn held(store: &SqliteStore) -> Vec<Held> {
    store.follow_up_holds().unwrap_or_default()
}

/// The conversation a held reminder now belongs on, once its message has left: the one its sent
/// copy landed in, or for a reply, the one it answers once the outbox has sent it.
pub(super) fn landed(store: &SqliteStore, held: &Held) -> Option<ThreadId> {
    if let Ok(Some(thread)) = store.thread_by_rfc_id(held.account.clone(), &held.message_id) {
        return Some(thread);
    }
    let sent = matches!(
        store.draft(held.draft).map(|draft| draft.state),
        Ok(SendState::Sent { .. })
    );
    held.thread.filter(|_| sent)
}
