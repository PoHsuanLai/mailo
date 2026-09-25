//! The property rows under the title: From, To, Cc, Sends, Protection and Attached. Every choice
//! is a [`Menu`] and every input a [`Field`].

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::data::account_rows;
use super::super::field::{Field, FieldKind};
use super::super::menu::{
    Floating, MenuItem, MenuKey, Right, Tile, anchor_at, menu_key, quire_entries,
};
use super::super::menus::{snooze_help, when_words};
use super::later::{PICK_KEY, PICK_LABEL, PickTime};
use super::page::{CcRow, Float, Guard, List, Page, PageKind, When};
use super::protection::ProtectionRow;
use super::receipt::{KEY as RECEIPT_KEY, ReceiptRow, item as receipt_item};
use super::recipients::{commit_typed, people_items, pick_person, pop_last, remove, typed};
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::provider::provider;
use crate::view::Shell;
use ds::{
    Anim, AvatarFace, AvatarShape, AvatarSize, AvatarTone, Button, ButtonVariant, Chip,
    ChipVariant, Glyph, Icon, MenuKind, MountedRef, PulseKey,
};

#[component]
pub(in crate::ui) fn Props(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    use_flash_clock(page);
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
                div { class: "k", Glyph { icon: Icon::Send, size: ds::IconSize::Compact }, "To" }
                div { class: "v",
                    Recipients { page, list: List::To }
                    if !cc_shown {
                        Button {
                            variant: ButtonVariant::Quiet,
                            label: "Cc".to_owned(),
                            onclick: move |_| page.write().cc_row = CcRow::Shown,
                        }
                    }
                }
            }
            if cc_shown {
                div { class: "prop-row", "data-row": "cc",
                    div { class: "k", Glyph { icon: Icon::Corner, size: ds::IconSize::Compact }, "Cc" }
                    div { class: "v", Recipients { page, list: List::Cc } }
                }
            }
            if !compact {
                SendsRow { page }
                ProtectionRow { page }
            }
            ReceiptRow { page }
            if !attached.is_empty() {
                div { class: "prop-row",
                    div { class: "k", Glyph { icon: Icon::Paperclip, size: ds::IconSize::Compact }, "Attached" }
                    div { class: "v",
                        for (index, (name, size)) in attached.into_iter().enumerate() {
                            Chip {
                                key: "{index}",
                                variant: ChipVariant::Neutral,
                                text: format!("{} · {name} · {size}", kind_of(&name)),
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
    // Local folders send nothing, so they are no From to choose.
    let items: Vec<MenuItem> = rows
        .iter()
        .filter(|row| !row.is_local())
        .map(|row| MenuItem {
            key: row.id.to_string(),
            tile: Tile::Avatar {
                letter: row
                    .address
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_ascii_uppercase(),
                color: super::super::command::avatar_color(&row.address),
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
    let mut value = use_signal(|| None::<MountedRef>);
    rsx! {
        div { class: "prop-row",
            div { class: "k", Glyph { icon: Icon::Mail, size: ds::IconSize::Compact }, "From" }
            div { class: "v",
                button {
                    class: "pval",
                    r#type: "button",
                    onmounted: move |event: MountedEvent| value.set(Some(MountedRef(event.data()))),
                    onclick: move |_| {
                        let next = if open { Float::Closed } else { Float::From };
                        page.write().float = next;
                    },
                    if let Some(via) = via {
                        ProvChip { provider: via, marks, place: ChipPlace::Inline }
                    }
                    "{address}"
                    span { class: "car", "▾" }
                }
                if open {
                    Floating {
                        kind: MenuKind::Dropdown,
                        anchor: value(),
                        title: "Send from".to_owned(),
                        items,
                        on_pick: move |key: String| move_to(page, &key),
                        on_close: move |_| close(page, &Float::From),
                    }
                }
            }
        }
    }
}

/// A menu closed: the page's float goes back to closed, unless its pick already opened another.
fn close(mut page: Signal<Page>, was: &Float) {
    if page.peek().float == *was {
        page.write().float = Float::Closed;
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

/// The Sends menu: right away, the two named times, "Pick a time…", and the receipt. A named
/// time's help is the date it resolves to, written as the snooze menu writes one.
pub(in crate::ui) fn sends_items<Tz: chrono::TimeZone>(
    page: &Page,
    now: chrono::DateTime<chrono::Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let when = page.when;
    let row = |key: &str, icon, name: String, help: String, on: bool| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name,
        help: Some(help),
        right: Right::Check(on),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    let mut items: Vec<MenuItem> = When::ALL
        .into_iter()
        .map(|choice| {
            let help = match choice.due(now, zone) {
                Some(at) => snooze_help(at, zone),
                None => "as soon as you press Send".to_owned(),
            };
            let icon = if choice == When::Now {
                Icon::Send
            } else {
                Icon::Clock
            };
            row(
                choice.key(),
                icon,
                choice.label().to_owned(),
                help,
                choice == when,
            )
        })
        .collect();
    let (help, on) = match when {
        When::At(at) => (when_words(at, now, zone), true),
        _ => ("tomorrow 9, fri 17:00, +2h".to_owned(), false),
    };
    items.push(row(PICK_KEY, Icon::Clock, PICK_LABEL.to_owned(), help, on));
    items.push(receipt_item(page.receipt));
    items
}

#[component]
fn SendsRow(page: Signal<Page>) -> Element {
    let when = page.read().when;
    let float = page.read().float.clone();
    let open = float == Float::Sends;
    let picking = matches!(float, Float::PickTime(_));
    let now = chrono::Utc::now();
    let items = sends_items(&page.read(), now, &chrono::Local);
    let shown = when.shown(now, &chrono::Local);
    let mut value = use_signal(|| None::<MountedRef>);
    rsx! {
        div { class: "prop-row",
            div { class: "k", Glyph { icon: Icon::Clock, size: ds::IconSize::Compact }, "Sends" }
            div { class: "v",
                button {
                    class: "pval",
                    r#type: "button",
                    onmounted: move |event: MountedEvent| value.set(Some(MountedRef(event.data()))),
                    onclick: move |_| {
                        let next = if open || picking { Float::Closed } else { Float::Sends };
                        page.write().float = next;
                    },
                    "{shown}"
                    if when != When::Now {
                        span { class: "mono", "scheduled" }
                    }
                    span { class: "car", "▾" }
                }
                if open {
                    Floating {
                        kind: MenuKind::Dropdown,
                        anchor: value(),
                        title: "Send".to_owned(),
                        items,
                        on_pick: move |key: String| {
                            pick_sends(&mut page.write(), &key);
                            if matches!(page.peek().float, Float::PickTime(_)) {
                                crate::ui::host::Host::focus_after_task(".pick-field");
                            }
                        },
                        // "Pick a time…" opens the field where the menu was: closing the menu
                        // after that pick must not close the field.
                        on_close: move |_| close(page, &Float::Sends),
                    }
                }
                if picking {
                    div { class: "p-menu", PickTime { page } }
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
    } else if key == PICK_KEY {
        // The field opens where the menu was; the choice is made when a time is entered.
        page.float = Float::PickTime(String::new());
        return;
    } else if key == RECEIPT_KEY {
        page.toggle_receipt();
    }
    page.float = Float::Closed;
}

/// A person who joins a list flashes their chip: `Page::flash` names them, and quire's timer
/// for `chip-flash` clears it once the flash has settled. The flash is set by whatever added
/// the person, so the timer starts when the name appears, not from a press here.
fn use_flash_clock(mut page: Signal<Page>) {
    let timer = ds::use_motion_timer(Anim::ChipFlash);
    let done = use_callback(move |()| page.write().flash = None);
    let mut started = use_signal(|| None::<String>);
    use_effect(move || {
        let flash = page.read().flash.clone();
        if flash.is_some() && flash != *started.peek() {
            timer.start(done);
        }
        if flash != *started.peek() {
            started.set(flash);
        }
    });
}

/// The chips of one list, the field beside them, and the people menu under it.
#[component]
fn Recipients(page: Signal<Page>, list: List) -> Element {
    // The field's box, which the people menu floats under.
    let mut field_at = use_signal(|| None::<ds::MountedRef>);
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
                let avatar = AvatarFace {
                    initial: person.name.chars().next().and_then(|ch| ch.to_uppercase().next()).unwrap_or('?'),
                    size: AvatarSize::Size18,
                    tone: AvatarTone::Person(ds::person_hue(&address)),
                    shape: AvatarShape::Round,
                };
                // The flash is the chip's own pulse, for as long as the page's flash timer runs.
                let pulse = flashing.then(|| PulseKey::rest(Anim::ChipFlash).fired());
                rsx! {
                    span {
                        key: "{address}",
                        Chip {
                            variant: ChipVariant::Person(avatar),
                            text: person.name.clone(),
                            onremove: move |()| remove(&mut page.write(), list, &address),
                            pulse,
                        }
                    }
                }
            }
        }
        div {
            class: "c-pin",
            onmounted: move |event| field_at.set(Some(ds::MountedRef(event.data()))),
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
                on_input: move |value: String| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    typed(&mut page.write(), list, value, store.as_ref());
                },
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
                    // quire's menu under the field, its cursor the field's: the field keeps the
                    // keyboard and picks with the row it knows.
                    ds::Menu::<String> {
                        kind: ds::MenuKind::Slim,
                        anchor: anchor_at(field_at()),
                        entries: quire_entries("From your contacts", &items, ds::AvatarSize::Size20, None),
                        onpick: move |key: String| pick_person(&mut page.write(), list, &key),
                        onclose: move |()| {
                            if matches!(page.peek().float, Float::People { list: open, .. } if open == list) {
                                page.write().float = Float::Closed;
                            }
                        },
                        active: ds::Cursor::Controlled(Some(active.min(items.len().saturating_sub(1)))),
                        on_active: move |to: Option<usize>| {
                            if let Some(to) = to {
                                page.write().float = Float::People { list, active: to };
                            }
                        },
                    }
                }
            }
        }
    }
}
