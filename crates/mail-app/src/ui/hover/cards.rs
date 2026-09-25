//! The layer that draws whichever card is open, and the cards that are only reading.
//!
//! Each reads what the store already holds and nothing else: a thread's row, its messages'
//! stored text, the sender history. See the module above for why that is the whole rule.

use super::super::history::{History, history};
use super::sender::sender_card;
use super::{Hook, keep_driver, use_driver};
use crate::space::{Pinned, Spaces};
use crate::view::Shell;
use chrono::Local;
use dioxus::prelude::*;
use ds::{AvatarTone, HoverCardPart, HoverMessage, Key, KeyHint, Shortcut};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The sender history, asked once per revision of the store and kept between hovers.
type Cached = Rc<RefCell<Option<(u64, Rc<History>)>>>;

fn cached(cache: &Cached, store: &SqliteStore, revision: u64) -> Rc<History> {
    if let Some((seen, known)) = cache.borrow().as_ref()
        && *seen == revision
    {
        return known.clone();
    }
    let fresh = Rc::new(history(store));
    *cache.borrow_mut() = Some((revision, fresh.clone()));
    fresh
}

/// Whichever card quire's hub has open (or is closing), as quire's `HoverCard`, placed by the
/// hub against its hook. Drawn once, inside the root.
#[component]
pub(in crate::ui) fn HoverLayer(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    spaces: Option<Signal<Spaces>>,
) -> Element {
    let cache: Cached = use_hook(Cached::default);
    let driver = use_driver();
    use_hook(move || keep_driver(driver));
    let Some(driver) = driver else {
        return rsx! {};
    };
    let hub = driver.hub();
    let Some((key, kind)) = hub.open().or(hub.leaving()) else {
        return rsx! {};
    };
    let Some(hook) = Hook::of(&key) else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let Some(Card { parts, more }) = (match hook {
        Hook::Thread(id) => thread_card(&store, id, &shell.read()),
        Hook::Sender(id) => {
            let known = cached(&cache, &store, revision());
            sender_card(&store, id, &known, shell, spaces)
        }
        Hook::Time(id) => time_tip(&store, id),
        Hook::Pin(index) => pin_card(&store, index, spaces, &shell.read()),
        Hook::Today(id) => today_card(&store, id),
    }) else {
        return rsx! {};
    };
    rsx! {
        ds::HoverCard { key: "{key.0}", kind, parts, {more} }
    }
}

/// A card's content: quire's parts, then whatever the parts do not draw, in mailo's box.
pub(super) struct Card {
    pub parts: Vec<HoverCardPart>,
    pub more: Element,
}

/// One message's first lines, from the text the store already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Lines {
    Text(String),
    /// Headers only: the body was never fetched, and hovering will not fetch it.
    NotDownloaded,
    /// A body with no plain text part.
    NoText,
}

/// The first lines of a stored body, quotes skipped, cut at about two lines of the card.
pub(in crate::ui) fn first_lines(body: &Body) -> Lines {
    match body {
        Body::Absent => Lines::NotDownloaded,
        Body::Present { text: None, .. } => Lines::NoText,
        Body::Present {
            text: Some(text), ..
        } => {
            let joined = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('>'))
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                return Lines::NoText;
            }
            let mut cut: String = joined.chars().take(160).collect();
            if cut.len() < joined.len() {
                cut.push('…');
            }
            Lines::Text(cut)
        }
    }
}

/// A name's first letter, upper-cased, as an avatar shows it.
pub(super) fn letter(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('?')
}

pub(super) fn who(address: &Address) -> String {
    address
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.email.clone())
}

/// A message in a card: its sender's letter on the ink, a name and its lines.
fn message(name: String, text: String) -> HoverMessage {
    HoverMessage {
        initial: letter(&name),
        tone: AvatarTone::Ink,
        name,
        text,
    }
}

fn thread_card(store: &SqliteStore, id: ThreadId, shell: &Shell) -> Option<Card> {
    let loaded = store.thread(id).ok()?;
    let summary = &loaded.summary;
    let count = loaded.messages.len();
    let plural = if count == 1 { "" } else { "s" };
    let labels: Vec<String> = summary
        .labels
        .iter()
        .filter_map(|label| {
            shell
                .labels
                .iter()
                .find(|(_, id)| id == label)
                .map(|(name, _)| name.clone())
        })
        .collect();
    let mut sub = format!("{count} message{plural} · {}", who(&summary.from));
    if !labels.is_empty() {
        sub.push_str(&format!(" · {}", labels.join(", ")));
    }
    let start = loaded.messages.len().saturating_sub(3);
    let recent: Vec<HoverMessage> = loaded.messages[start..]
        .iter()
        .filter_map(|message| store.message(*message).ok())
        .map(|stored| {
            let text = match first_lines(&stored.body) {
                Lines::Text(text) => text,
                Lines::NotDownloaded => "not downloaded".to_owned(),
                Lines::NoText => "no plain text".to_owned(),
            };
            message(who(&stored.from), text)
        })
        .collect();
    let unread = summary.read == ReadState::Unread;
    let foot = if unread {
        "stays unread while you look"
    } else {
        "already read"
    };
    Some(Card {
        parts: vec![
            HoverCardPart::Title(summary.subject.clone()),
            HoverCardPart::Sub(sub),
            HoverCardPart::Messages(recent),
            HoverCardPart::Foot {
                text: foot.to_owned(),
                keys: Some(KeyHint {
                    shortcut: Shortcut(vec![Key::Space]),
                    label: "peek".to_owned(),
                }),
            },
        ],
        more: rsx! {},
    })
}

fn time_tip(store: &SqliteStore, id: ThreadId) -> Option<Card> {
    let loaded = store.thread(id).ok()?;
    let full = loaded
        .summary
        .last_date
        .with_timezone(&Local)
        .format("%A %-d %B %Y, %H:%M")
        .to_string();
    Some(Card {
        parts: Vec::new(),
        more: rsx! {
            div { class: "hc",
                "{full}"
                div { class: "sub", "your time" }
            }
        },
    })
}

fn pin_card(
    store: &SqliteStore,
    index: usize,
    spaces: Option<Signal<Spaces>>,
    shell: &Shell,
) -> Option<Card> {
    let pin = spaces.and_then(|spaces| spaces.read().current_space().pins.get(index).cloned())?;
    let (name, filter) = match &pin {
        Pinned::Person { name, email } => (
            name.clone(),
            Filter::From(TextMatch::Contains(email.clone())),
        ),
        Pinned::Search { name, query } => (
            name.clone(),
            crate::query::parse_with(query, &Local, &crate::query::named(&shell.labels)),
        ),
    };
    let query = Query {
        filter: Filter::And(vec![Filter::Read(ReadState::Unread), filter]),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 3,
        },
    };
    let unread = store
        .threads(&query, chrono::Utc::now())
        .map(|page| page.items)
        .unwrap_or_default();
    let sub = if unread.is_empty() {
        "nothing unread".to_owned()
    } else {
        format!("latest {} unread", unread.len())
    };
    let mut parts = vec![HoverCardPart::Title(name), HoverCardPart::Sub(sub)];
    if !unread.is_empty() {
        parts.push(HoverCardPart::Messages(
            unread
                .into_iter()
                .map(|thread| HoverMessage {
                    initial: letter(&who(&thread.from)),
                    tone: AvatarTone::Ink,
                    name: thread.subject,
                    text: thread.snippet,
                })
                .collect(),
        ));
    }
    Some(Card {
        parts,
        more: rsx! {},
    })
}

fn today_card(store: &SqliteStore, id: ThreadId) -> Option<Card> {
    let loaded = store.thread(id).ok()?;
    let summary = loaded.summary;
    let from = who(&summary.from);
    Some(Card {
        parts: vec![
            HoverCardPart::Title(summary.subject),
            HoverCardPart::Sub(format!("{from} · opened today")),
            HoverCardPart::Messages(vec![HoverMessage {
                initial: letter(&from),
                tone: AvatarTone::Ink,
                name: String::new(),
                text: summary.snippet,
            }]),
        ],
        more: rsx! {},
    })
}
