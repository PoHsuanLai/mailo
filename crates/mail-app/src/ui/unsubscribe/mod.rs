//! Leaving a mailing list from the reader: whether the open thread offers a way out, what taking
//! it will do, and taking it.
//!
//! The way out is in the stored raw message, so finding it reads a blob. That is done once per
//! thread, off the thread that draws, and only for the thread the reader has open: the lookup is
//! reached from [`head::Leave`] and from nowhere on the list's or the hover cards' paths, and a
//! test counts the reads of the app's [`Looks`] to keep it that way. A thread whose body has not been fetched offers
//! nothing here — fetching it to find out would be a POP3 `RETR`, which marks it read.
//!
//! What each way out does is [`mail_core::unsubscribe`]'s, the module `mailo unsubscribe` uses, so
//! the window and the command cannot disagree about it. A web page is shown and never opened.

mod head;

pub(super) use head::Leave;

use super::motion::Follow;
use super::ops;
use crate::ui::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::message::Looks;
use mail_core::undo::Undo;
use mail_core::unsubscribe::{Found, Outcome};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_mime::Unsubscribe;

// A thread's way out of its list, with the name the popover and the toast call the list by.
pub(in crate::ui) use mail_core::message::Offer;

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
                format!("Leave {list}?")
            }
            Ask::Mailto { list, to, from } => {
                format!("Leave {list}? A message goes to {to} from {from}.")
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

// What a thread's answer is a function of: its messages and the bytes each one has.
pub(in crate::ui) use mail_core::message::Bodies;

/// The answer already found for `thread` at `key`, if one has been. `Some(None)` is a thread
/// that was looked at and offers nothing.
pub(in crate::ui) fn cached(
    looks: &Looks,
    thread: ThreadId,
    key: &Bodies,
) -> Option<Option<Offer>> {
    looks.offer_cached(thread, key)
}

/// [`cached`], else read from the stored mail and remembered. This reads a blob: it runs on a
/// blocking thread, for the thread the reader has open, and nowhere else.
pub(in crate::ui) fn lookup(
    looks: &Looks,
    store: &SqliteStore,
    thread: ThreadId,
    key: &Bodies,
) -> Option<Offer> {
    looks.offer(store, thread, key)
}

/// Take the preferred way out, through [`mail_core::unsubscribe::perform`]. Blocking: the window
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
        None => return Err("No way to unsubscribe.".to_owned()),
        Some(Unsubscribe::Web { url }) => return Ok(Outcome::Page { url: url.clone() }),
        Some(Unsubscribe::OneClick { .. } | Unsubscribe::Mailto(_)) => {}
    }
    let http = http()?;
    // This is a blocking thread, where waiting on a future needs the application's runtime.
    crate::edge::block_on(mail_core::unsubscribe::perform(store, found, &http, now))
        .map_err(String::from)
}

/// The client a one-click `POST` is made with.
pub(in crate::ui) fn client() -> Result<reqwest::Client, String> {
    mail_core::unsubscribe::client().map_err(|e| e.to_string())
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

/// The toast's "Archive all from this list": [`archive_from`], kept for ⌘Z one at a time,
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
        0 => format!("Nothing left from {list}"),
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
