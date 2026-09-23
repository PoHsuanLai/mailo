//! The layer that draws whichever card is open, and the cards that are only reading.
//!
//! Each reads what the store already holds and nothing else: a thread's row, its messages'
//! stored text, the sender history. See the module above for why that is the whole rule.

use super::super::history::{History, history};
use super::sender::sender_card;
use super::{Hook, hover};
use crate::space::{Pinned, Spaces};
use crate::view::Shell;
use chrono::Local;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Which cards a layer draws. The thread card sits beside the list, so it is drawn inside the
/// list column; the rest are positioned against the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Site {
    List,
    Frame,
}

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

/// Whatever card is open, if it belongs to this site.
#[component]
pub(in crate::ui) fn HoverLayer(
    site: Site,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    spaces: Option<Signal<Spaces>>,
) -> Element {
    let cache: Cached = use_hook(Cached::default);
    let Some(state) = hover() else {
        return rsx! {};
    };
    let Some((hook, (x, y))) = state.open() else {
        return rsx! {};
    };
    let mine = matches!(hook, Hook::Thread(_)) == (site == Site::List);
    if !mine {
        return rsx! {};
    }
    let store = consume_context::<Arc<SqliteStore>>();
    let (class, style) = match hook {
        Hook::Thread(_) => ("hc beside", format!("top:{:.0}px", (y - 12.0).max(8.0))),
        Hook::Sender(_) => ("hc", format!("left:{:.0}px;top:{:.0}px", x, y + 22.0)),
        Hook::Time(_) => (
            "hc tip",
            format!(
                "left:max(8px, calc({x:.0}px - 140px));top:{:.0}px",
                y + 20.0
            ),
        ),
        Hook::Pin(_) | Hook::Today(_) => (
            "hc side-card",
            format!("left:244px;top:{:.0}px", (y - 6.0).max(8.0)),
        ),
    };
    let body = match hook {
        Hook::Thread(id) => thread_card(&store, id, &shell.read()),
        Hook::Sender(id) => {
            let known = cached(&cache, &store, revision());
            sender_card(&store, id, &known, shell, spaces)
        }
        Hook::Time(id) => time_tip(&store, id),
        Hook::Pin(index) => pin_card(&store, index, spaces, &shell.read()),
        Hook::Today(id) => today_card(&store, id),
    };
    rsx! {
        div {
            class: "{class}",
            style: "{style}",
            role: "tooltip",
            onpointerenter: move |_| state.enter_card(),
            onpointerleave: move |_| state.leave_card(),
            {body}
        }
    }
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

pub(super) fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_owned())
}

pub(super) fn who(address: &Address) -> String {
    address
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.email.clone())
}

fn thread_card(store: &SqliteStore, id: ThreadId, shell: &Shell) -> Element {
    let Ok(loaded) = store.thread(id) else {
        return rsx! {};
    };
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
    let recent: Vec<(String, String, Lines)> = loaded.messages[start..]
        .iter()
        .filter_map(|message| store.message(*message).ok())
        .map(|message| {
            let name = who(&message.from);
            (initial(&name), name, first_lines(&message.body))
        })
        .collect();
    let unread = summary.read == ReadState::Unread;
    let subject = summary.subject.clone();
    rsx! {
        h5 { "{subject}" }
        div { class: "sub", "{sub}" }
        div { class: "msgs",
            for (n, (letter, name, lines)) in recent.into_iter().enumerate() {
                div { key: "{n}", class: "msg",
                    span { class: "av", "{letter}" }
                    div {
                        b { "{name}" }
                        match lines {
                            Lines::Text(text) => rsx! { p { "{text}" } },
                            Lines::NotDownloaded => rsx! { p { class: "none", "not downloaded" } },
                            Lines::NoText => rsx! { p { class: "none", "no plain text" } },
                        }
                    }
                }
            }
        }
        div { class: "foot",
            if unread { "stays unread while you look" } else { "already read" }
            span { class: "k", kbd { "Space" } " peek" }
        }
    }
}

fn time_tip(store: &SqliteStore, id: ThreadId) -> Element {
    let Ok(loaded) = store.thread(id) else {
        return rsx! {};
    };
    let full = loaded
        .summary
        .last_date
        .with_timezone(&Local)
        .format("%A %-d %B %Y, %H:%M")
        .to_string();
    rsx! {
        "{full}"
        div { class: "sub", "your time" }
    }
}

fn pin_card(
    store: &SqliteStore,
    index: usize,
    spaces: Option<Signal<Spaces>>,
    shell: &Shell,
) -> Element {
    let Some(pin) =
        spaces.and_then(|spaces| spaces.read().current_space().pins.get(index).cloned())
    else {
        return rsx! {};
    };
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
    rsx! {
        h5 { "{name}" }
        div { class: "sub", "{sub}" }
        if !unread.is_empty() {
            div { class: "msgs",
                for thread in unread {
                    div { key: "{thread.id}", class: "msg",
                        span { class: "av", "{initial(&who(&thread.from))}" }
                        div {
                            b { "{thread.subject}" }
                            p { "{thread.snippet}" }
                        }
                    }
                }
            }
        }
    }
}

fn today_card(store: &SqliteStore, id: ThreadId) -> Element {
    let Ok(loaded) = store.thread(id) else {
        return rsx! {};
    };
    let summary = loaded.summary;
    let from = who(&summary.from);
    rsx! {
        h5 { "{summary.subject}" }
        div { class: "sub", "{from} · opened today" }
        div { class: "msgs",
            div { class: "msg",
                span { class: "av", "{initial(&from)}" }
                div { p { "{summary.snippet}" } }
            }
        }
    }
}
