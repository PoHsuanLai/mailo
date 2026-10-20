//! The header rows between the subject and the body, as a message's header is set: one
//! label column of one width starting on the pane's leading edge, no colons, every value at one x.
//!
//! A new message draws To and nothing else. Cc and Bcc are drawn once added (their keys, or their
//! words at the end of the To row) or while someone is on them; From only when more than one
//! address can send; Sends, Remind, Security and Receipt only while they are not the default, each
//! with a menu that sets it back ("Now", "Off", "None"). The choices themselves live under Send ▾
//! and the foot's ⋯ (`foot.rs`). Every choice is a quire `Menu` and every input a quire
//! `TextField`.

use super::super::press::on_primary;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_marks::Leading;
use ds::components::controls::button_model::Bezel;
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::components::fields::text_field_model::Invalid;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_core::SqliteStore;

use super::super::data::account_rows;
use super::super::menu::{
    Floating, MenuItem, MenuKey, Right, Tile, anchor_at, menu_items, menu_key,
};
use super::super::menus::snooze_help;
use super::later::{PICK_KEY, PICK_LABEL};
use super::page::{Float, Guard, List, Page, PageKind, When};
use super::protection::ProtectionRow;
use super::receipt::ReceiptRow;
use super::recipients::{commit_typed, people_items, pick_person, pop_last, remove, typed};
use super::remind::RemindRow;
use crate::ui::actions::{self, Own};
use crate::ui::copy;
use crate::ui::provider_chip::ProvChip;
use crate::ui::view::Shell;
use mail_core::provider::provider;

#[component]
pub(in crate::ui) fn Props(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    let read = page.read();
    let new = read.kind == PageKind::New;
    let cc = read.row_shown(List::Cc);
    let bcc = read.row_shown(List::Bcc);
    let guard = read.guard;
    drop(read);
    let refused = match guard {
        Guard::NoRecipient(count) => Some(count),
        _ => None,
    };
    let store = consume_context::<Arc<SqliteStore>>();
    // One address to send from is no choice: the row is drawn only when there is another.
    let senders = mail_core::compose::sending_accounts(&store).len();
    rsx! {
        div { class: "c-props",
            if new && senders > 1 {
                FromRow { page, shell }
            }
            Line {
                label: list_label(List::To),
                row: "to",
                Recipients { page, list: List::To, refused }
                if !cc {
                    Reveal { page, list: List::Cc }
                }
                if !bcc {
                    Reveal { page, list: List::Bcc }
                }
            }
            if cc {
                Line {
                    label: list_label(List::Cc),
                    row: "cc",
                    Recipients { page, list: List::Cc, refused: None }
                }
            }
            if bcc {
                Line {
                    label: list_label(List::Bcc),
                    row: "bcc",
                    Recipients { page, list: List::Bcc, refused: None }
                }
            }
            SendsRow { page }
            RemindRow { page }
            ProtectionRow { page }
            ReceiptRow { page }
        }
    }
}

/// What a recipient list's row and field are called.
pub(in crate::ui) fn list_label(list: List) -> &'static str {
    match list {
        List::To => "To",
        List::Cc => "Cc",
        List::Bcc => "Bcc",
    }
}

/// The key that adds `list`'s row, when it has one.
pub(in crate::ui) fn reveal_key(list: List) -> Option<Own> {
    match list {
        List::To => None,
        List::Cc => Some(Own::AddCc),
        List::Bcc => Some(Own::AddBcc),
    }
}

/// The input of `list`'s field, for the keyboard to go to once the row is drawn.
fn field_of(list: List) -> &'static str {
    match list {
        List::To => ".c-props [*|data-row=to] .pinput input",
        List::Cc => ".c-props [*|data-row=cc] .pinput input",
        List::Bcc => ".c-props [*|data-row=bcc] .pinput input",
    }
}

/// Draw `list`'s row and put the keyboard in it: its word at the end of the To row, or its key.
pub(in crate::ui) fn reveal(mut page: Signal<Page>, list: List) {
    page.write().reveal(list);
    crate::ui::host::Host::focus_after_task(field_of(list));
}

/// "Cc" or "Bcc" at the end of the To row, until that row is drawn.
#[component]
fn Reveal(page: Signal<Page>, list: List) -> Element {
    let name = list_label(list);
    rsx! {
        Button {
            bezel: Bezel::Inline,
            label: name.to_owned(),
            title: format!("Add {name}"),
            title_shortcut: reveal_key(list).and_then(actions::tip_own),
            onclick: on_primary(move || reveal(page, list)),
            common: Common {
                extra_class: ExtraClass::parse("c-reveal").ok(),
                aria_label: Some(format!("Add {name}")),
                ..Common::default()
            },
        }
    }
}

/// One header line: its name, small and secondary, on the label column's leading edge, then its
/// value or control from the value column's edge, which every line shares. Its `data-row` is what
/// the page's tests and its layout find it by.
#[component]
pub(super) fn Line(#[props(into)] label: String, row: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "c-line", role: "group", aria_label: "{label}", "data-row": row,
            Label {
                text: label.clone(),
                role: LabelRole::Secondary,
                style: LabelStyle::Footnote,
                common: crate::ui::common::classed("c-line-k"),
            }
            div { class: "c-line-v", {children} }
        }
    }
}

/// A row that says one choice which is not the default, and the menu to change it, whose first
/// item sets it back: Sends, Remind, Security and Receipt. The menu is open while the page's
/// float is `float`.
#[component]
pub(super) fn ChoiceRow(
    page: Signal<Page>,
    label: &'static str,
    row: &'static str,
    value: String,
    float: Float,
    items: Vec<MenuItem>,
    on_pick: EventHandler<String>,
) -> Element {
    let open = page.read().float == float;
    let mut at = use_signal(|| None::<MountedRef>);
    let closing = float.clone();
    let toggling = float.clone();
    rsx! {
        Line {
            label,
            row,
            Button {
                bezel: Bezel::Inline,
                label: value.clone(),
                trailing: Some(Icon::ChevronDown),
                shown: Some(if open { Shown::Visible } else { Shown::Hidden }),
                onclick: on_primary(move || {
                    let next = if open { Float::Closed } else { toggling.clone() };
                    page.write().float = next;
                }),
                common: Common {
                    aria_label: Some(format!("{label}: {value}")),
                    mounted: Some(EventHandler::new(move |event: MountedEvent| at.set(Some(MountedRef(event.data()))))),
                    ..Common::default()
                },
            }
            if open {
                Floating {
                    placement: MenuPlacement::Popup,
                    anchor: at(),
                    title: String::new(),
                    items,
                    on_pick,
                    on_close: move |_| close(page, &closing),
                }
            }
        }
    }
}

#[component]
fn FromRow(page: Signal<Page>, shell: Signal<Shell>) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let from = page.read().from.clone();
    let open = page.read().float == Float::From;
    let rows = account_rows(&store);
    let marks = shell.read().appearance.marks;
    let current = rows.iter().find(|row| row.id == from);
    let address = current.map(|row| row.address.clone()).unwrap_or_default();
    // A provider's mark says which one only when the addresses are on more than one.
    let mut providers: Vec<&str> = rows
        .iter()
        .filter(|row| !row.is_local())
        .map(|row| provider(&row.plan).title())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    let via = current
        .filter(|_| providers.len() > 1)
        .map(|row| provider(&row.plan));
    // Local folders send nothing, so they are no From to choose.
    let items: Vec<MenuItem> = rows
        .iter()
        .filter(|row| !row.is_local())
        .map(|row| MenuItem {
            tile: Tile::Avatar {
                letter: row
                    .address
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_ascii_uppercase(),
                color: super::super::command::avatar_color(&row.address),
            },
            help: Some(provider(&row.plan).title().to_owned()),
            right: Right::Check(row.id == from),
            ..MenuItem::plain(row.id.to_string(), row.address.clone())
        })
        .collect();
    let mut value = use_signal(|| None::<MountedRef>);
    rsx! {
        Line {
            label: copy::fields::FROM,
            row: "from",
            Button {
                bezel: Bezel::Inline,
                label: address,
                // The provider's mark leads the value, inside it: quire's `ProviderMark` at its
                // inline size, drawn from the cached icon or the letter.
                leading: via.map(|via| Leading::Mark(rsx! {
                    ProvChip { provider: via, marks }
                })),
                trailing: Some(Icon::ChevronDown),
                shown: Some(if open { Shown::Visible } else { Shown::Hidden }),
                onclick: on_primary(move || {
                    let next = if open { Float::Closed } else { Float::From };
                    page.write().float = next;
                }),
                common: Common {
                    mounted: Some(EventHandler::new(move |event: MountedEvent| value.set(Some(MountedRef(event.data()))))),
                    ..Common::default()
                },
            }
            if open {
                Floating {
                    placement: MenuPlacement::Popup,
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
    let moved = super::life::save(&store, &mut write, now).and_then(|draft| {
        mail_core::compose::move_draft_to(&store, draft.id, account, now)
            .map_err(crate::ui::remedy::told)
    });
    match moved {
        Ok(draft) => write.from = draft.account,
        Err(why) => write.notice = Some(why),
    }
}

/// The Sends row's menu: "Now" first, the way back to the default, then the two named times and
/// "Pick a Time…". A named time's hint is the date it resolves to, as the snooze menu writes one.
pub(in crate::ui) fn sends_items<Tz: chrono::TimeZone>(
    page: &Page,
    now: chrono::DateTime<chrono::Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let when = page.when;
    let row = |key: &str, icon, name: String, help: Option<String>, on: bool| MenuItem {
        help,
        right: Right::Check(on),
        ..MenuItem::icon(key, icon, name)
    };
    let mut items: Vec<MenuItem> = When::ALL
        .into_iter()
        .map(|choice| {
            let help = choice.due(now, zone).map(|at| snooze_help(at, zone));
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
        When::At(at) => (Some(super::super::menus::when_words(at, now, zone)), true),
        _ => (None, false),
    };
    items.push(row(PICK_KEY, Icon::Clock, PICK_LABEL.to_owned(), help, on));
    items
}

/// The Sends row, only while the message is scheduled: when it leaves, and the menu that sends it
/// "Now" instead.
#[component]
fn SendsRow(page: Signal<Page>) -> Element {
    let when = page.read().when;
    if when == When::Now {
        return rsx! {};
    }
    let now = chrono::Utc::now();
    let items = sends_items(&page.read(), now, &chrono::Local);
    rsx! {
        ChoiceRow {
            page,
            label: "Sends",
            row: "sends",
            value: when.shown(now, &chrono::Local),
            float: Float::Sends,
            items,
            on_pick: move |key: String| {
                let mut page = page;
                pick_sends(&mut page.write(), &key);
            },
        }
    }
}

/// A choice from the Sends row's menu or Send ▾ › Send Later: when it goes. "Pick a Time…" opens
/// the field by Send ▾; the choice is made when a time is entered there.
pub(in crate::ui) fn pick_sends(page: &mut Page, key: &str) {
    if let Some(choice) = When::from_key(key) {
        page.when = choice;
        page.touch();
    } else if key == PICK_KEY {
        page.float = Float::PickTime(String::new());
        return;
    }
    page.float = Float::Closed;
}

/// The files on the draft, as one strip at the end of the body: each its name and size.
#[component]
pub(super) fn Attached(page: Signal<Page>) -> Element {
    let attached = page.read().attached.clone();
    if attached.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "c-attached", role: "group", aria_label: "Attachments", "data-row": "attached",
            for (index, (name, size)) in attached.into_iter().enumerate() {
                Chip {
                    key: "{index}-{name}",
                    variant: ChipVariant::Neutral,
                    text: format!("{} · {name} · {size}", kind_of(&name)),
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

/// The chips of one list, the field beside them, and the people menu under it.
#[component]
fn Recipients(page: Signal<Page>, list: List, refused: Option<u32>) -> Element {
    // To takes the keyboard as a new, unaddressed message opens (`super::opening`).
    let focus = use_hook(|| match (list, super::opening(&page.peek())) {
        (List::To, super::Opening::To) => FieldFocus::OnMount,
        _ => FieldFocus::Manual,
    });
    // The field's box, which the people menu floats under.
    let mut field_at = use_signal(|| None::<MountedRef>);
    let read = page.read();
    let people = read.list(list).to_vec();
    let value = read.typed(list).to_owned();
    let menu = match read.float {
        Float::People { list: open, active } if open == list => Some(active),
        _ => None,
    };
    let items = people_items(&read, list);
    // What is typed that is not an address says so first: mended, the field may still be the
    // empty To a send was refused for.
    let validity = match (read.misread(list), refused) {
        (Some(misread), _) => Validity::Invalid(Invalid {
            message: misread.why.clone().into(),
            stamp: EventStamp(misread.count),
        }),
        (None, Some(count)) => Validity::Invalid(Invalid {
            message: copy::errors::ADD_A_RECIPIENT.into(),
            stamp: EventStamp(count),
        }),
        (None, None) => Validity::Valid,
    };
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
                let avatar = AvatarFace {
                    initial: person.name.chars().next().and_then(|ch| ch.to_uppercase().next()).unwrap_or('?'),
                    size: AvatarSize::Size18,
                    tone: AvatarTone::Person(person_hue(&address)),
                    shape: AvatarShape::Round,
                };
                rsx! {
                    span {
                        key: "{address}",
                        Chip {
                            variant: ChipVariant::Person(avatar),
                            text: person.name.clone(),
                            onremove: move |()| remove(&mut page.write(), list, &address),
                        }
                    }
                }
            }
        }
        div {
            class: "c-pin",
            onmounted: move |event| field_at.set(Some(MountedRef(event.data()))),
            onkeydown: move |event: KeyboardEvent| {
                // Only a key that changes the page writes it: a write is a render, and one under
                // every key the field types would race the text the field is editing.
                let key = event.key().to_string();
                match (menu_key(&key), menu) {
                    (Some(MenuKey::Down), Some(active)) => {
                        page.write().float = Float::People { list, active: active + 1 };
                        event.prevent_default();
                    }
                    (Some(MenuKey::Up), Some(active)) => {
                        page.write().float = Float::People { list, active: active.saturating_sub(1) };
                        event.prevent_default();
                    }
                    (Some(MenuKey::Enter), Some(active)) => {
                        let mut write = page.write();
                        let items = people_items(&write, list);
                        if let Some(item) = items.get(active.min(items.len().saturating_sub(1))) {
                            pick_person(&mut write, list, &item.key);
                        }
                        event.prevent_default();
                    }
                    (Some(MenuKey::Enter), None) => {
                        commit_typed(&mut page.write(), list);
                        event.prevent_default();
                    }
                    (Some(MenuKey::Escape), Some(_)) => {
                        page.write().float = Float::Closed;
                        event.stop_propagation();
                    }
                    // Backspace in an empty field takes the last chip; in a field with text it
                    // is the field's.
                    (Some(MenuKey::Backspace), _)
                        if page.peek().typed(list).is_empty() && !page.peek().list(list).is_empty() =>
                    {
                        pop_last(&mut page.write(), list);
                    }
                    _ => {}
                }
            },
            TextField {
                label: list_label(list).to_owned(),
                bezel: FieldBezel::Plain,
                placeholder: placeholder.to_owned(),
                focus,
                validity,
                value,
                oninput: move |value: String| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    typed(&mut page.write(), list, value, store.as_ref());
                },
                onblur: move |_| {
                    let mut write = page.write();
                    if write.float == (Float::People { list, active: menu.unwrap_or(0) }) {
                        return;
                    }
                    commit_typed(&mut write, list);
                },
                common: Common { extra_class: ExtraClass::parse("pinput").ok(), ..Common::default() },
            }
            if let Some(active) = menu {
                if !items.is_empty() {
                    // quire's menu under the field, its cursor the field's: the field keeps the
                    // keyboard and picks with the row it knows.
                    Menu::<String> {
                        placement: MenuPlacement::Popup,
                        anchor: anchor_at(field_at()),
                        items: menu_items("From your contacts", &items, false),
                        onpick: move |key: String| pick_person(&mut page.write(), list, &key),
                        onclose: move |()| {
                            if matches!(page.peek().float, Float::People { list: open, .. } if open == list) {
                                page.write().float = Float::Closed;
                            }
                        },
                        active: MenuCursor::Controlled(Some(active.min(items.len().saturating_sub(1)))),
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
