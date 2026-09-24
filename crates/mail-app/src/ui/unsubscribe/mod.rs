//! Leaving a mailing list from the reader: whether the open thread offers a way out, what taking
//! it will do, and taking it.
//!
//! The way out is in the stored raw message, so finding it reads a blob. That is done once per
//! thread, off the thread that draws, and only for the thread the reader has open: [`look`] is
//! reached from [`head::Leave`] and from nowhere on the list's or the hover cards' paths, and a
//! test counts its calls to keep it that way. A thread whose body has not been fetched offers
//! nothing here — fetching it to find out would be a POP3 `RETR`, which marks it read.
//!
//! What each way out does is [`crate::unsubscribe`]'s, the module `mailo unsubscribe` uses, so
//! the window and the command cannot disagree about it. A web page is shown and never opened.

mod head;

pub(super) use head::Leave;

use super::motion::Follow;
use super::ops;
use crate::undo::Undo;
use crate::unsubscribe::{Found, Outcome};
use crate::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::*;
use mail_mime::Unsubscribe;
use mail_store::{SqliteStore, Store};

/// A thread's way out of its list, with what the popover says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Offer {
    pub found: Found,
    /// The list's name, as the popover and the toast say it.
    pub list: String,
    /// Who the list's mail comes from, for "Archive all from this list".
    pub sender: Address,
    /// The address a `mailto:` unsubscribe leaves from.
    pub from: Address,
}

/// What the confirm popover says will happen, from the preferred way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Ask {
    /// An RFC 8058 `POST`, made by this client.
    OneClick { list: String },
    /// A message to the list's software, queued like any send.
    Mailto {
        list: String,
        to: String,
        from: String,
    },
    /// A page, which this client shows and never opens.
    Web { url: String },
}

impl Ask {
    /// The sentence the popover leads with.
    pub(in crate::ui) fn sentence(&self) -> String {
        match self {
            Ask::OneClick { list } => {
                format!("Leave {list}? mailo sends the list's server a one-click request.")
            }
            Ask::Mailto { list, to, from } => {
                format!("Leave {list}? mailo sends a message to {to} from {from}.")
            }
            Ask::Web { .. } => "This list can only be left on its web page.".to_owned(),
        }
    }

    /// The button that takes the way out, when this client takes it. A page has none.
    pub(in crate::ui) fn action(&self) -> Option<&'static str> {
        match self {
            Ask::OneClick { .. } => Some("Unsubscribe"),
            Ask::Mailto { .. } => Some("Send"),
            Ask::Web { .. } => None,
        }
    }
}

/// The name a list goes by: its `List-Id` description, its id, else whoever sends it.
fn list_name(found: &Found, sender: &Address) -> String {
    match &found.list.id {
        Some(id) => id.description.clone().unwrap_or_else(|| id.id.clone()),
        None => sender
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| sender.email.clone()),
    }
}

/// The offer for `found`, or `None` when it has no way out this client can use.
pub(in crate::ui) fn offer_of(found: Found, sender: Address, from: Address) -> Option<Offer> {
    found.list.preferred()?;
    Some(Offer {
        list: list_name(&found, &sender),
        found,
        sender,
        from,
    })
}

/// What the popover says for `offer`.
pub(in crate::ui) fn ask(offer: &Offer) -> Option<Ask> {
    let list = offer.list.clone();
    Some(match offer.found.list.preferred()? {
        Unsubscribe::OneClick { .. } => Ask::OneClick { list },
        Unsubscribe::Mailto(mailto) => Ask::Mailto {
            list,
            to: mailto
                .to
                .iter()
                .map(|to| to.email.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            from: offer.from.email.clone(),
        },
        Unsubscribe::Web { url } => Ask::Web { url: url.clone() },
    })
}

#[cfg(test)]
static LOOKED: std::sync::Mutex<Vec<ThreadId>> = std::sync::Mutex::new(Vec::new());

/// Every thread [`look`] has read a blob for, in this test process. Thread ids are fresh in each
/// fixture, so a test asks about its own and is not confused by another running beside it.
#[cfg(test)]
pub(in crate::ui) fn looked_at(thread: ThreadId) -> bool {
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .contains(&thread)
}

/// Read the thread's way out of its list from the stored mail. This reads a blob: it runs on a
/// blocking thread, for the thread the reader has open, and nowhere else.
pub(in crate::ui) fn look(store: &SqliteStore, thread: ThreadId) -> Option<Offer> {
    #[cfg(test)]
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .push(thread);
    // Only stored bytes are read: a message whose body is not here is skipped, never fetched.
    let found = crate::unsubscribe::find(store, *thread.as_uuid()).ok()?;
    let sender = store.message(found.message).ok()?.from;
    let from = crate::compose::address_addressed(store, found.account, &found.addressed).ok()?;
    offer_of(found, sender, from)
}

/// What an answer is a function of: the thread's messages and the bytes each one has. A body
/// that arrives, or a message that joins the thread, is a new key; nothing else changes it,
/// because a blob is content-addressed.
pub(in crate::ui) type Bodies = Vec<(MessageId, Option<BlobId>)>;

/// How many threads' answers to keep. An answer is a few short strings.
const KEPT: usize = 64;

static CACHE: std::sync::Mutex<Vec<(ThreadId, Bodies, Option<Offer>)>> =
    std::sync::Mutex::new(Vec::new());

/// The answer already found for `thread` at `key`, if one has been. `Some(None)` is a thread
/// that was looked at and offers nothing.
pub(in crate::ui) fn cached(thread: ThreadId, key: &Bodies) -> Option<Option<Offer>> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(had, at, _)| *had == thread && at == key)
        .map(|(_, _, offer)| offer.clone())
}

/// [`cached`], else [`look`], remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(store: &SqliteStore, thread: ThreadId, key: &Bodies) -> Option<Offer> {
    if let Some(had) = cached(thread, key) {
        return had;
    }
    let offer = look(store, thread);
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(had, _, _)| *had != thread);
    cache.push((thread, key.clone(), offer.clone()));
    if cache.len() > KEPT {
        cache.remove(0);
    }
    offer
}

/// Take the preferred way out, through [`crate::unsubscribe::perform`]. Blocking: the window
/// calls it on a blocking thread.
///
/// `http` is asked for a client only when one could be used. A web page returns before it is
/// called, so leaving by a page cannot make a request: no client is ever built for it.
pub(in crate::ui) fn leave(
    store: &SqliteStore,
    found: &Found,
    now: DateTime<Utc>,
    http: impl FnOnce() -> Result<reqwest::Client, String>,
) -> Result<Outcome, String> {
    match found.list.preferred() {
        None => return Err("This message offers no way to unsubscribe.".to_owned()),
        Some(Unsubscribe::Web { url }) => return Ok(Outcome::Page { url: url.clone() }),
        Some(Unsubscribe::OneClick { .. } | Unsubscribe::Mailto(_)) => {}
    }
    let http = http()?;
    // Its own runtime, as `mailo unsubscribe` does: this is a blocking thread, where waiting on
    // a future needs one, and the window's runtime is not this thread's to block.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Could not start the request: {e}"))?;
    runtime.block_on(crate::unsubscribe::perform(store, found, &http, now))
}

/// The client a one-click `POST` is made with.
pub(in crate::ui) fn client() -> Result<reqwest::Client, String> {
    mail_runtime::unsubscribe::client().map_err(|e| e.to_string())
}

/// Archive every conversation in the inbox from `sender`, as ordinary ops with their undos.
///
/// By sender rather than by `List-Id`: no filter can ask for a list, because the id is not a
/// column — it is read from the raw message, which is exactly the per-row blob read this avoids.
/// A list sends from one address, so `from:` it is the same set of conversations in practice.
pub(in crate::ui) fn archive_from(store: &SqliteStore, sender: &str) -> Vec<Undo> {
    let query = Query {
        filter: Filter::And(vec![
            Filter::From(TextMatch::Exact(sender.to_owned())),
            Filter::InMailbox(MailboxRole::Inbox),
        ]),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 500,
        },
    };
    let Ok(page) = store.threads(&query, Utc::now()) else {
        return Vec::new();
    };
    page.items
        .iter()
        .filter_map(|thread| ops::perform(store, thread.id, Op::Archive))
        .collect()
}

/// The toast's "Archive all from this list": [`archive_from`], kept for Ctrl Z one at a time,
/// and said.
pub(in crate::ui) fn archive_list(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
    sender: &str,
    list: &str,
) {
    let undone = archive_from(store, sender);
    let count = undone.len();
    for undo in undone {
        shell.write().undo.push(undo);
    }
    revision += 1;
    let text = match count {
        0 => format!("Nothing from {list} is left in the inbox"),
        1 => format!("Archived 1 conversation from {list}"),
        n => format!("Archived {n} conversations from {list}"),
    };
    super::motion::tell(text, Follow::Nothing);
}

/// What the toast says once the way out is taken.
pub(in crate::ui) fn said(outcome: &Outcome, list: &str) -> String {
    match outcome {
        Outcome::Unsubscribed { .. } => format!("Unsubscribed from {list}"),
        Outcome::Queued { .. } => "Unsubscribe message queued".to_owned(),
        Outcome::Page { .. } => "This list can only be left on its web page".to_owned(),
    }
}

#[cfg(test)]
mod tests;
