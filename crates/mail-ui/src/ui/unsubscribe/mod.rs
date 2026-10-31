//! Leaving a mailing list from the reader: whether the open thread offers a way out, what taking
//! it will do, and taking it.
//!
//! The way out is in the stored raw message, so finding it reads a blob. That is done once per
//! thread, off the thread that draws, and only for the thread the reader has open: the lookup is
//! reached from [`head::Lookout`] and from nowhere on the list's or the hover cards' paths, and a
//! test counts the reads of the app's [`Looks`] to keep it that way. A thread whose body has not been fetched offers
//! nothing here — fetching it to find out would be a POP3 `RETR`, which marks it read.
//!
//! What each way out does is [`mail_client::unsubscribe`]'s, the module `mailo unsubscribe` uses, so
//! the window and the command cannot disagree about it. A web page is shown and never opened.

mod head;

pub(in crate::ui) use head::{Confirm, Lookout, Phase};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_client::Client;
use mail_client::message::Looks;
use mail_client::undo::Undo;
use mail_client::unsubscribe::{Found, Outcome};
use mail_client::{Acted, Perform};
use mail_domain::*;
use mail_mime::Unsubscribe;
use mail_ui_kit::view::Shell;

// A thread's way out of its list, with the name the alert and the toast call the list by.
pub(in crate::ui) use mail_client::message::Offer;

/// What the confirm alert says will happen, from the preferred way out.
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
    /// The sentence the alert leads with.
    pub(in crate::ui) fn sentence(&self) -> String {
        match self {
            Ask::OneClick { list } => {
                format!("Leave {list}?")
            }
            Ask::Mailto { list, to, from } => {
                format!("Leave {list}? This sends a message to {to} from {from}.")
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

/// What the alert says for `offer`.
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
pub(in crate::ui) use mail_client::message::Bodies;

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
    client: &Client,
    thread: ThreadId,
    key: &Bodies,
) -> Option<Offer> {
    client.looking(looks).offer(thread, key)
}

/// Take the preferred way out, through [`mail_client::unsubscribe::perform`]. Blocking: the window
/// calls it on a blocking thread.
///
/// `http` is asked for a client only when one could be used. A web page returns before it is
/// called, so leaving by a page cannot make a request: no client is ever built for it.
pub(in crate::ui) fn leave(
    client: &Client,
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
    client
        .net()
        .unsubscribe(found, &http, now)
        .map_err(String::from)
}

/// The client a one-click `POST` is made with.
pub(in crate::ui) fn http() -> Result<reqwest::Client, String> {
    mail_client::unsubscribe::client().map_err(|e| e.to_string())
}

/// Every conversation in the inbox from `sender`.
///
/// By sender rather than by `List-Id`: no filter can ask for a list, because the id is not a
/// column — it is read from the raw message, which is exactly the per-row blob read this avoids.
/// A list sends from one address, so `from:` it is the same set of conversations in practice.
fn inbox_from(client: &Client, sender: &str) -> Vec<ThreadId> {
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
    let Ok(page) = client.stored().threads(&query, Utc::now()) else {
        return Vec::new();
    };
    page.items.iter().map(|thread| thread.id).collect()
}

/// Archive every conversation in the inbox from `sender`, here, as ordinary ops with their
/// undos.
#[cfg(test)]
pub(in crate::ui) fn archive_from(client: &Client, sender: &str) -> Vec<Undo> {
    inbox_from(client, sender)
        .into_iter()
        .filter_map(|thread| client.acting().perform(thread, Op::Archive))
        .collect()
}

/// The toast's "Archive all from this list": every conversation in the inbox from `sender`
/// archived as one gesture, kept as one undo for ⌘Z and the toast, and said.
pub(in crate::ui) fn archive_list(client: &Client, shell: Signal<Shell>, sender: &str, list: &str) {
    let ops = inbox_from(client, sender)
        .into_iter()
        .map(|thread| (thread, Op::Archive))
        .collect();
    let motion = mail_ui_kit::motion::motion();
    let gesture = mail_ui_kit::motion::begin(motion);
    let ticket = mail_ui_kit::live::client().act(Perform {
        ops,
        alone: Vec::new(),
        then_erase: false,
    });
    let list = list.to_owned();
    mail_ui_kit::motion::when_done(ticket, move |acted| {
        let undone: Vec<Undo> = match acted {
            Some(Acted { done, .. }) => done.into_iter().map(|(.., undo)| undo).collect(),
            None => Vec::new(),
        };
        let count = undone.len();
        let text = match count {
            0 => format!("Nothing left from {list}"),
            1 => format!("Archived 1 conversation from {list}"),
            n => format!("Archived {n} conversations from {list}"),
        };
        let kept = mail_ui_kit::motion::record_through(motion, shell, undone, text);
        mail_ui_kit::motion::end(motion, shell, gesture, kept);
    });
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
