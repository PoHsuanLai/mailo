//! Leaving a mailing list: which way the message offers, and taking it.
//!
//! The list headers are read from the stored raw message each time they are asked for, not
//! kept in a column. The raw blob is already the single source of truth for everything a
//! message says (`Body::Present::raw`), reading only the header block of one message is cheap,
//! and a column would need a migration, a change to the frozen `Message`, and a backfill of
//! every message already stored — all to cache something asked for once per list, by hand.
//!
//! What each way out means here:
//! - one-click: an HTTPS `POST`, made now by [`mail_runtime::unsubscribe::one_click`];
//! - a `mailto:`: a draft from the account the message came to, queued through the ordinary
//!   send path, so it leaves on the next sync like any other message and can be seen in
//!   `mailo drafts` until it has;
//! - a web page: reported, never fetched. Opening it is the person's decision.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::{ListHeaders, Mailto, Unsubscribe};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// A message's way out of its list, with what is needed to take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The message the headers were read from.
    pub message: MessageId,
    /// The account it came to, which is the one a `mailto:` unsubscribe is sent from.
    pub account: AccountId,
    /// Everyone it was addressed to, for choosing which of the account's identities is on the list.
    pub addressed: Vec<Address>,
    pub list: ListHeaders,
}

/// What taking the way out did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The list's server accepted the one-click `POST`.
    Unsubscribed { url: String },
    /// An unsubscribe message is queued, and leaves on the next sync.
    Queued { draft: DraftId },
    /// Only a page is offered, and this client does not open pages. Here is its address.
    Page { url: String },
}

/// The list headers of one message.
///
/// An error only when there is nothing to read them from: the body has not been fetched yet
/// (only its headers were, and those were not kept), or the stored bytes are gone.
pub fn list_of(store: &SqliteStore, message: &Message) -> Result<ListHeaders, CoreError> {
    let raw = message.body.raw().ok_or(CoreError::NotFetchedInFull)?;
    let bytes = store
        .blobs()
        .get(raw)
        .map_err(|e| CoreError::context("the stored message is unreadable", e))?;
    Ok(mail_mime::list_headers(&bytes))
}

/// The way out offered by a message, or by the newest message in a thread that offers one.
///
/// `id` is tried as a message first and then as a thread, because both print as a uuid and the
/// person should not have to say which one they copied.
pub fn find(store: &SqliteStore, id: uuid::Uuid) -> Result<Found, CoreError> {
    if let Ok(message) = store.message(MessageId::from_uuid(id)) {
        return found(store, &message);
    }
    let thread = store
        .thread(ThreadId::from_uuid(id))
        .map_err(|_| CoreError::NeitherMessageNorThread(id))?;
    let mut messages = thread
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect::<Vec<_>>();
    // Newest first: a list that moved its unsubscribe address says so in its latest mail.
    messages.sort_by_key(|message| std::cmp::Reverse(message.date));
    let mut first = None;
    for message in &messages {
        let Ok(had) = found(store, message) else {
            continue;
        };
        if !had.list.unsubscribe.is_empty() {
            return Ok(had);
        }
        first.get_or_insert(had);
    }
    first.ok_or(CoreError::ThreadNotFetchedInFull)
}

fn found(store: &SqliteStore, message: &Message) -> Result<Found, CoreError> {
    Ok(Found {
        message: message.id,
        account: message.account.clone(),
        addressed: message.to.iter().chain(&message.cc).cloned().collect(),
        list: list_of(store, message)?,
    })
}

/// Queue the message a `mailto:` asks for, from the account that received the list's mail.
///
/// Through [`crate::compose::send`], the same path every message takes: the bytes are frozen
/// and the outbox delivers them on the next sync. A URI with no subject gets `unsubscribe`,
/// which is what list software that reads subjects looks for and harmless to the rest.
pub fn queue_mailto(
    store: &SqliteStore,
    found: &Found,
    mailto: &Mailto,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let identity =
        crate::compose::identity_addressed(store, found.account.clone(), &found.addressed);
    let subject = if mailto.subject.trim().is_empty() {
        "unsubscribe"
    } else {
        mailto.subject.as_str()
    };
    let draft = crate::compose::draft_exact(
        store,
        found.account.clone(),
        identity,
        &mailto.to,
        subject,
        &mailto.body,
        now,
    )?;
    crate::compose::send(store, draft.id, now)?;
    Ok(store.draft(draft.id)?)
}

/// Take the preferred way out of the list.
///
/// `http` is a client from [`mail_runtime::unsubscribe::client`]; it is only used for one-click.
pub async fn perform(
    store: &SqliteStore,
    found: &Found,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Outcome, CoreError> {
    match found.list.preferred() {
        None => Err(CoreError::NothingOffered),
        Some(Unsubscribe::OneClick { url }) => {
            mail_runtime::unsubscribe::one_click(http, url).await?;
            Ok(Outcome::Unsubscribed {
                url: url.to_string(),
            })
        }
        Some(Unsubscribe::Mailto(mailto)) => {
            let draft = queue_mailto(store, found, mailto, now)?;
            Ok(Outcome::Queued { draft: draft.id })
        }
        Some(Unsubscribe::Web { url }) => Ok(Outcome::Page { url: url.clone() }),
    }
}
