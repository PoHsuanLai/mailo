//! The property rows under the title: From, To, Cc, Sends and Attached. Every choice is a
//! [`Menu`] and every input a [`Field`].

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::data::account_rows;
use super::super::field::{Field, FieldKind};
use super::super::icon::{Glyph, Icon};
use super::super::menu::{Menu, MenuItem, MenuKey, Right, Tile, menu_key};
use super::float::hue;
use super::page::{CcRow, Float, Guard, List, Page, PageKind, When};
use super::receipt::{KEY as RECEIPT_KEY, ReceiptRow, item as receipt_item};
use super::recipients::{commit_typed, people_items, pick_person, pop_last, remove, typed};
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::provider::provider;
use crate::view::Shell;

#[component]
pub(in crate::ui) fn Props(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    let read = page.read();
    let compact = read.kind == PageKind::Reply;
    let cc_shown = read.cc_row == CcRow::Shown;
    let shaking = match read.guard {
        Guard::Shake(count) => Some(count),
        _ => None,
    };
    let attached = read.attached.clone();
    drop(read);
    rsx! {
        div { class: "c-props",
            if !compact {
                FromRow { page, shell }
            }
            div {
                // Two names for one animation, so a second press starts it again.
                class: match shaking {
                    None => "prop-row",
                    Some(count) if count % 2 == 1 => "prop-row shake",
                    Some(_) => "prop-row shake again",
                },
                "data-row": "to",
                div { class: "k", Glyph { icon: Icon::Send, class: None }, "To" }
                div { class: "v",
                    Recipients { page, list: List::To }
                    if !cc_shown {
                        button {
                            class: "plink",
                            r#type: "button",
                            onclick: move |_| page.write().cc_row = CcRow::Shown,
                            "Cc"
                        }
                    }
                }
            }
            if cc_shown {
                div { class: "prop-row", "data-row": "cc",
                    div { class: "k", Glyph { icon: Icon::Corner, class: None }, "Cc" }
                    div { class: "v", Recipients { page, list: List::Cc } }
                }
            }
            if !compact {
                SendsRow { page }
            }
            ReceiptRow { page }
            if !attached.is_empty() {
                div { class: "prop-row",
                    div { class: "k", Glyph { icon: Icon::Paperclip, class: None }, "Attached" }
                    div { class: "v",
                        for (index, (name, size)) in attached.into_iter().enumerate() {
                            span { key: "{index}", class: "pchip",
                                span { class: "av file", "{kind_of(&name)}" }
                                "{name}"
                                span { class: "mono size", "{size}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The short kind a file chip shows: its extension, upper-cased.
fn kind_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.chars().take(4).collect::<String>().to_uppercase())
        .unwrap_or_else(|| "FILE".to_owned())
}

#[component]
fn FromRow(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let from = page.read().from;
    let open = page.read().float == Float::From;
    let rows = account_rows(&store);
    let marks = shell.read().appearance.marks;
    let current = rows.iter().find(|row| row.id == from);
    let address = current.map(|row| row.address.clone()).unwrap_or_default();
    let via = current.map(|row| provider(&row.plan));
    let items: Vec<MenuItem> = rows
        .iter()
        .map(|row| MenuItem {
            key: row.id.to_string(),
            tile: Tile::Avatar {
                letter: row
                    .address
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_ascii_uppercase(),
                color: crate::space::AVATAR[hue(&row.address) % crate::space::AVATAR.len()]
                    .to_owned(),
            },
            name: row.address.clone(),
            help: Some(provider(&row.plan).title().to_owned()),
            right: Right::Check(row.id == from),
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect();
    rsx! {
        div { class: "prop-row",
            div { class: "k", Glyph { icon: Icon::Mail, class: None }, "From" }
            div { class: "v",
                button {
                    class: "pval",
                    r#type: "button",
                    onclick: move |_| {
                        let next = if open { Float::Closed } else { Float::From };
                        page.write().float = next;
                    },
                    if let Some(via) = via {
                        ProvChip { provider: via, marks, place: ChipPlace::Row }
                    }
                    "{address}"
                    span { class: "car", "▾" }
                }
                if open {
                    div { class: "p-menu",
                        Menu {
                            title: "Send from".to_owned(),
                            items,
                            filterable: false,
                            on_pick: move |key: String| move_to(page, &key),
                            on_close: move |_| page.write().float = Float::Closed,
                            on_query: move |_| {},
                            slim: true,
                            active: None,
                        }
                    }
                }
            }
        }
    }
}

/// Move the draft to the account `key` names. Saved first, so nothing typed is read back over.
fn move_to(mut page: Signal<Page>, key: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    let now = chrono::Utc::now();
    let mut write = page.write();
    write.float = Float::Closed;
    let Some(account) = account_rows(&store)
        .into_iter()
        .find(|row| row.id.to_string() == key)
        .map(|row| row.id)
    else {
        return;
    };
    let moved = super::life::save(&store, &mut write, now)
        .and_then(|draft| crate::compose::move_draft_to(&store, draft.id, account, now));
    match moved {
        Ok(draft) => write.from = draft.account,
        Err(why) => write.notice = Some(why),
    }
}

#[component]
fn SendsRow(page: Signal<Page>) -> Element {
    let when = page.read().when;
    let receipt = page.read().receipt;
    let open = page.read().float == Float::Sends;
    let mut items: Vec<MenuItem> = When::ALL
        .into_iter()
        .map(|choice| MenuItem {
            key: choice.key().to_owned(),
            tile: Tile::Icon(if choice == When::Now {
                Icon::Send
            } else {
                Icon::Clock
            }),
            name: choice.label().to_owned(),
            help: Some(choice.help().to_owned()),
            right: Right::Check(choice == when),
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect();
    items.push(receipt_item(receipt));
    rsx! {
        div { class: "prop-row",
            div { class: "k", Glyph { icon: Icon::Clock, class: None }, "Sends" }
            div { class: "v",
                button {
                    class: "pval",
                    r#type: "button",
                    onclick: move |_| {
                        let next = if open { Float::Closed } else { Float::Sends };
                        page.write().float = next;
                    },
                    "{when.label()}"
                    if when != When::Now {
                        span { class: "mono", "scheduled" }
                    }
                    span { class: "car", "▾" }
                }
                if open {
                    div { class: "p-menu",
                        Menu {
                            title: "Send".to_owned(),
                            items,
                            filterable: false,
                            on_pick: move |key: String| pick_sends(&mut page.write(), &key),
                            on_close: move |_| page.write().float = Float::Closed,
                            on_query: move |_| {},
                            slim: true,
                            active: None,
                        }
                    }
                }
            }
        }
    }
}

/// A choice from the Sends menu: when it goes, or whether it asks for a read receipt.
pub(in crate::ui) fn pick_sends(page: &mut Page, key: &str) {
    if let Some(choice) = When::from_key(key) {
        page.when = choice;
        page.touch();
    } else if key == RECEIPT_KEY {
        page.toggle_receipt();
    }
    page.float = Float::Closed;
}

/// The chips of one list, the field beside them, and the people menu under it.
#[component]
fn Recipients(page: Signal<Page>, list: List) -> Element {
    let read = page.read();
    let people = match list {
        List::To => read.to.clone(),
        List::Cc => read.cc.clone(),
    };
    let value = match list {
        List::To => read.typed_to.clone(),
        List::Cc => read.typed_cc.clone(),
    };
    let flash = read.flash.clone();
    let menu = match read.float {
        Float::People { list: open, active } if open == list => Some(active),
        _ => None,
    };
    let items = people_items(&read, list);
    let placeholder = if people.is_empty() {
        "Add a person"
    } else {
        ""
    };
    drop(read);
    rsx! {
        for person in people {
            {
                let address = person.address.clone();
                let flashing = flash.as_deref() == Some(address.as_str());
                let letter = person.name.chars().next().map(|ch| ch.to_uppercase().collect::<String>()).unwrap_or_default();
                let color = crate::space::AVATAR[hue(&address) % crate::space::AVATAR.len()];
                rsx! {
                    span {
                        key: "{address}",
                        class: if flashing { "pchip flash" } else { "pchip" },
                        onanimationend: move |_| {
                            if flashing {
                                page.write().flash = None;
                            }
                        },
                        span { class: "av", style: "background:{color}", "{letter}" }
                        "{person.name}"
                        button {
                            class: "x",
                            r#type: "button",
                            aria_label: "Remove {person.name}",
                            onclick: move |_| remove(&mut page.write(), list, &address),
                            Glyph { icon: Icon::X, class: None }
                        }
                    }
                }
            }
        }
        div {
            class: "c-pin",
            onkeydown: move |event: KeyboardEvent| {
                let key = event.key().to_string();
                let mut write = page.write();
                match (menu_key(&key), menu) {
                    (Some(MenuKey::Down), Some(active)) => {
                        write.float = Float::People { list, active: active + 1 };
                        event.prevent_default();
                    }
                    (Some(MenuKey::Up), Some(active)) => {
                        write.float = Float::People { list, active: active.saturating_sub(1) };
                        event.prevent_default();
                    }
                    (Some(MenuKey::Enter), Some(active)) => {
                        let items = people_items(&write, list);
                        if let Some(item) = items.get(active.min(items.len().saturating_sub(1))) {
                            pick_person(&mut write, list, &item.key);
                        }
                        event.prevent_default();
                    }
                    (Some(MenuKey::Enter), None) => {
                        commit_typed(&mut write, list);
                        event.prevent_default();
                    }
                    (Some(MenuKey::Escape), Some(_)) => {
                        write.float = Float::Closed;
                        event.stop_propagation();
                    }
                    (Some(MenuKey::Backspace), _) => pop_last(&mut write, list),
                    _ => {}
                }
            },
            Field {
                kind: FieldKind::Inline,
                value,
                placeholder: placeholder.to_owned(),
                extra: Some("pinput".to_owned()),
                on_input: move |value: String| typed(&mut page.write(), list, value),
                on_focus: |_| {},
                on_blur: move |_| {
                    let mut write = page.write();
                    if write.float == (Float::People { list, active: menu.unwrap_or(0) }) {
                        return;
                    }
                    commit_typed(&mut write, list);
                },
            }
            if let Some(active) = menu {
                if !items.is_empty() {
                    div { class: "p-menu",
                        Menu {
                            title: "People you have written with".to_owned(),
                            items: items.clone(),
                            filterable: false,
                            on_pick: move |key: String| pick_person(&mut page.write(), list, &key),
                            on_close: move |_| page.write().float = Float::Closed,
                            on_query: move |_| {},
                            slim: true,
                            active: Some(active.min(items.len().saturating_sub(1))),
                        }
                    }
                }
            }
        }
    }
}
