//! A draft's copy in the server's Drafts mailbox: what is queued to replace it, and to remove it.
//!
//! Apple Mail and Thunderbird keep the draft being written in Drafts on the server, replace it as
//! it changes and remove it once it is sent or thrown away, so a reply started on one device can
//! be finished on another. The copy is the server's alone: it is never brought in here as a
//! message (Drafts is not followed by a sync), and the draft this client holds stays the
//! document. Every copy carries the draft's `Message-ID` ([`mail_mime::draft_message_id`]), which
//! is how each operation finds the ones before it.

use crate::RuntimeError;
use chrono::{DateTime, Utc};
use mail_domain::{BlobId, ChangeId, Draft, MailboxRef, Patch, RemoteIntent};
use mail_store::{SqliteStore, Store};

/// The `Message-ID` every copy of `draft` carries, without angle brackets; `None` when the
/// identity it is written from is gone, and with it the domain the id was made with.
pub fn message_id(store: &SqliteStore, draft: &Draft) -> Result<Option<String>, RuntimeError> {
    Ok(store
        .identity(draft.identity)?
        .map(|identity| mail_mime::draft_message_id(draft, &identity)))
}

/// Queue `raw`, the bytes of `draft` as it now stands, to replace its copy in the Drafts mailbox
/// at `mailbox`. An upload of it still waiting goes (`Store::enqueue`).
pub fn queue_keep(
    store: &SqliteStore,
    draft: &Draft,
    mailbox: MailboxRef,
    raw: BlobId,
    now: DateTime<Utc>,
) -> Result<bool, RuntimeError> {
    let Some(message_id) = message_id(store, draft)? else {
        return Ok(false);
    };
    queue(
        store,
        draft,
        RemoteIntent::KeepDraft {
            draft: draft.id,
            mailbox,
            message_id,
            raw,
        },
        now,
    )
}

/// Queue the removal of every copy of `draft` from the Drafts mailbox at `mailbox`: it was sent,
/// or discarded. An upload of it still waiting goes too.
pub fn queue_drop(
    store: &SqliteStore,
    draft: &Draft,
    mailbox: MailboxRef,
    now: DateTime<Utc>,
) -> Result<bool, RuntimeError> {
    let Some(message_id) = message_id(store, draft)? else {
        return Ok(false);
    };
    queue(
        store,
        draft,
        RemoteIntent::DropDraft {
            draft: draft.id,
            mailbox,
            message_id,
        },
        now,
    )
}

fn queue(
    store: &SqliteStore,
    draft: &Draft,
    intent: RemoteIntent,
    now: DateTime<Utc>,
) -> Result<bool, RuntimeError> {
    let queued = store.enqueue(
        draft.account.clone(),
        intent,
        // Nothing to undo: the copy changes nothing here, before or after it is made.
        &Patch {
            id: ChangeId::generate(),
            changes: Vec::new(),
        },
        now,
    )?;
    Ok(queued.is_some())
}
