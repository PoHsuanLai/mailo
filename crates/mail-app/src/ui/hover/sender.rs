//! The sender card: who they are, how often they write, and whether their name borrows a
//! brand their address does not belong to. Its actions are the shared `Menu`.

use super::super::contacts::ContactPart;
use super::super::history::History;
use super::super::menu::{Floating, MenuItem, Right, Tile};
use super::cards::{Card, letter, who};
use super::dismiss;
use crate::space::{Pinned, Spaces};
use crate::trust::spoof;
use crate::view::Shell;
use chrono::Local;
use dioxus::prelude::*;
use ds::{AvatarTone, Glyph, HoverCardPart, HoverStat, Icon};
use mail_domain::ThreadId;
use mail_store::{SqliteStore, Store};

pub(super) fn sender_card(
    store: &SqliteStore,
    id: ThreadId,
    known: &History,
    mut shell: Signal<Shell>,
    spaces: Option<Signal<Spaces>>,
) -> Option<Card> {
    let loaded = store.thread(id).ok()?;
    let from = loaded.summary.from.clone();
    let name = who(&from);
    let email = from.email.clone();
    let seen = known.get(&email).cloned();
    let first = seen.as_ref().is_none_or(|sender| sender.threads <= 1);
    let flag = spoof(from.name.as_deref(), &email);
    let threads = seen.as_ref().map_or(1, |sender| sender.threads);
    let last = seen
        .as_ref()
        .map(|sender| crate::view::listed(sender.last, chrono::Utc::now(), &Local))
        .unwrap_or_default();
    let items = sender_actions();
    let given = from.name.clone().unwrap_or_default();
    let parts = vec![
        HoverCardPart::Person {
            initial: letter(&name),
            tone: AvatarTone::Ink,
            title: name.clone(),
            sub: Some(email.clone()),
        },
        HoverCardPart::Stats(vec![
            HoverStat {
                value: threads.to_string(),
                label: if threads == 1 { "thread" } else { "threads" }.to_owned(),
            },
            HoverStat {
                value: last,
                label: "last wrote".to_owned(),
            },
        ]),
    ];
    // The flags bold the brand and the domain, which quire's `Flag` part (plain text) cannot:
    // they stay mailo's, under the parts, with the contact row and the actions.
    let more = rsx! {
        div { class: "hc",
        if let Some(flag) = flag {
            div { class: "flag",
                Glyph { icon: Icon::X, size: ds::IconSize::Compact }
                span {
                    "The name says "
                    b { "{flag.brand}" }
                    "; the address is "
                    b { "{flag.domain}" }
                    ", which is not one of theirs."
                }
            }
        }
        if first {
            div { class: "flag info",
                Glyph { icon: Icon::Mail, size: ds::IconSize::Compact }
                span { "First mail from this address. Nothing else in the store has come from it." }
            }
        }
        // Keyed, so a card for another sender starts afresh rather than keep this one's state.
        {rsx! { ContactPart { key: "{email}", email: email.clone(), name: given } }}
        div { class: "acts",
            // The card's actions are a menu drawn in the card, not over it.
            Floating {
                kind: ds::MenuKind::Slim,
                anchor: None,
                title: String::new(),
                items,
                flow: ds::Flow::Inline,
                // No row under a cursor: the card is pointed at, never arrowed through, and a
                // first row drawn as selected reads as one already chosen.
                active: ds::Cursor::Controlled(None),
                on_pick: move |key: String| {
                    match key.as_str() {
                        "pin" => pin_person(spaces, &name, &email),
                        "mail" => shell.write().search = format!("from:{email}"),
                        _ => copy(&email),
                    }
                    dismiss();
                },
                on_close: move |_| dismiss(),
            }
        }
        }
    };
    Some(Card { parts, more })
}

fn sender_actions() -> Vec<MenuItem> {
    [
        ("pin", Icon::Star, "Pin to sidebar"),
        ("mail", Icon::Search, "Their mail"),
        ("copy", Icon::Mail, "Copy address"),
    ]
    .into_iter()
    .map(|(key, icon, name)| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: None,
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}

/// Add the sender to the current Space's pinned people, once.
fn pin_person(spaces: Option<Signal<Spaces>>, name: &str, email: &str) {
    let Some(mut spaces) = spaces else {
        return;
    };
    let current = spaces.peek().current;
    {
        let mut write = spaces.write();
        let Some(space) = write.spaces.get_mut(current) else {
            return;
        };
        let already = space.pins.iter().any(|pin| {
            matches!(pin, Pinned::Person { email: known, .. } if known.eq_ignore_ascii_case(email))
        });
        if !already {
            space.pins.push(Pinned::Person {
                name: name.to_owned(),
                email: email.to_owned(),
            });
        }
    }
    super::super::frame::keep(&spaces.read());
}

/// Put an address on the clipboard.
pub(in crate::ui) fn copy(email: &str) {
    crate::ui::host::Host::copy(email);
}
