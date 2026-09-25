//! Account tiles, places, labels and pinned people.

use super::super::data::account_rows;
use super::super::hover::{Hook, element, out, over, use_driver};
use super::super::motion::{drag, motion};
use crate::provider::icon::{mark_of, mark_style};
use crate::provider::{Provider, provider};
use crate::query::{self};
use crate::space::{self, Pinned, Scope, Space};
use crate::view::{Shell, Source, folder_of, is_label_place};
use dioxus::prelude::*;
use ds::{
    Anim, AvatarFace, AvatarShape, AvatarSize, AvatarTone, Colour, DropState, Here, Hex, Icon,
    ItemKind, PersonSwatch, PlaceId, Presence, PulseKey, SidebarItem,
};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Counts {
    /// Each account's id, its name as the tile says it, its unread count, and its provider —
    /// none for local folders, which are on no provider.
    pub rows: Vec<(AccountId, String, u64, Option<Provider>)>,
    pub all: u64,
    pub pins: Vec<u64>,
}

pub(super) fn counts(store: &SqliteStore, space: &Space) -> Counts {
    let rows = account_rows(store);
    let shown: Vec<_> = rows
        .into_iter()
        .filter(|row| match &space.scope {
            Scope::All => true,
            Scope::Accounts(ids) => ids.contains(&row.id),
        })
        .collect();
    let labels = query::known_labels(store);
    let scope = scope_filter(space);
    let now = chrono::Utc::now();
    let unread = |extra: Filter| {
        let mut parts = vec![Filter::Read(ReadState::Unread), extra];
        if let Some(scope) = scope.clone() {
            parts.push(scope);
        }
        store.count(&Filter::And(parts), now).unwrap_or(0)
    };
    let all = match &scope {
        Some(scope) => store
            .count(
                &Filter::And(vec![Filter::Read(ReadState::Unread), scope.clone()]),
                now,
            )
            .unwrap_or(0),
        None => store
            .count(&Filter::Read(ReadState::Unread), now)
            .unwrap_or(0),
    };
    let rows = shown
        .iter()
        .map(|row| {
            let n = store
                .count(
                    &Filter::And(vec![
                        Filter::Read(ReadState::Unread),
                        Filter::Account(row.id),
                    ]),
                    now,
                )
                .unwrap_or(0);
            let via = (!row.is_local()).then(|| provider(&row.plan));
            (row.id, row.shown(), n, via)
        })
        .collect();
    let pins = space
        .pins
        .iter()
        .map(|pin| unread(pin_filter(pin, &labels)))
        .collect();
    Counts { rows, all, pins }
}

fn scope_filter(space: &Space) -> Option<Filter> {
    match &space.scope {
        Scope::All => None,
        Scope::Accounts(ids) if ids.len() == 1 => Some(Filter::Account(ids[0])),
        Scope::Accounts(ids) if ids.is_empty() => Some(Filter::Nothing),
        Scope::Accounts(ids) => Some(Filter::Or(
            ids.iter().copied().map(Filter::Account).collect(),
        )),
    }
}

fn pin_filter(pin: &Pinned, labels: &[(String, LabelId)]) -> Filter {
    match pin {
        Pinned::Person { email, .. } => Filter::From(TextMatch::Contains(email.clone())),
        Pinned::Search { query, .. } => {
            query::parse_with(query, &chrono::Local, &query::named(labels))
        }
    }
}

fn initial(text: &str) -> char {
    super::today::initial(text)
}

/// A stored `#rrggbb` as quire's colour. A Space file hand-edited to something else draws in
/// the first swatch rather than failing.
pub(in crate::ui) fn hex_colour(text: &str) -> Colour {
    Colour::Solid(Hex::parse(text).unwrap_or_else(|| PersonSwatch::nth(0).hex()))
}

fn place_icon(name: &str) -> Icon {
    match name {
        "Inbox" => Icon::Inbox,
        "Starred" => Icon::Star,
        "Snoozed" => Icon::Clock,
        "Archive" => Icon::Archive,
        "Trash" => Icon::Trash,
        "Drafts" => Icon::FilePen,
        "Sent" => Icon::Send,
        "Spam" => Icon::OctagonAlert,
        "Pinned" => Icon::Pin,
        _ => Icon::Tag,
    }
}

#[component]
pub(super) fn AccountTiles(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    space: Space,
    counted: Counts,
) -> Element {
    let several = counted.rows.len() > 1;
    let marks = shell.read().appearance.marks;
    let pressed = |on: bool| if on { ds::Switch::On } else { ds::Switch::Off };
    rsx! {
        div { class: "pins", role: "group", aria_label: "Accounts in this Space",
            if several {
                ds::AccountTile {
                    account: ds::AccountFace::All,
                    pressed: pressed(shell.read().account.is_none()),
                    unread: count_of(counted.all),
                    onclick: move |()| {
                        shell.write().account = None;
                        pages.set(1);
                    },
                }
            }
            for (index, row) in counted.rows.iter().enumerate() {
                {
                    let id = row.0;
                    let address = row.1.clone();
                    let n = row.2;
                    let on = !several || shell.read().account == Some(id);
                    let colour = hex_colour(&space::avatar_color(&space, id, index));
                    let letter = initial(&address);
                    let mut pick = move |()| {
                        shell.write().account = Some(id);
                        pages.set(1);
                    };
                    // Local folders are on no provider: quire's neutral folder mark.
                    let (provider, mark) = match row.3 {
                        Some(via) => (mark_of(via), mark_style(via, marks)),
                        None => (ds::Provider::Local, ds::MarkStyle::Letter),
                    };
                    rsx! {
                        ds::AccountTile {
                            key: "{id}",
                            account: ds::AccountFace::One {
                                initial: letter,
                                colour,
                                provider,
                                address: Some(address),
                            },
                            pressed: pressed(on),
                            unread: count_of(n),
                            onclick: move |()| pick(()),
                            mark,
                        }
                    }
                }
            }
            ds::AddAccountTile {
                title: "Add account…".to_owned(),
                onclick: move |()| super::super::add_account::open(shell),
            }
        }
    }
}

/// An unread count as a tile's badge holds it.
fn count_of(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[component]
pub(super) fn PlaceList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    folded: Vec<LabelId>,
) -> Element {
    let places = shell.read().places.clone();
    // Labels and then folders follow the default places; folders are drawn under Folders.
    let split = places
        .iter()
        .position(|place| is_label_place(place) || folder_of(place).is_some())
        .unwrap_or(places.len());
    // A label that is also a mailbox is drawn once, under Folders. See `folder_tree::arrange`.
    let labels: Vec<(usize, String)> = places
        .iter()
        .enumerate()
        .skip(split)
        .filter(|(_, place)| match &place.source {
            Source::Mail(Filter::HasLabel(id)) => !folded.contains(id),
            _ => false,
        })
        .map(|(index, place)| (index, place.name.clone()))
        .collect();
    rsx! {
        div { class: "s-h", "Places" }
        for (index, place) in places.iter().take(split).enumerate() {
            PlaceButton { index, name: place.name.clone(), icon: place_icon(&place.name), shell, pages, badges }
        }
        if !labels.is_empty() {
            div { class: "s-h", "Labels" }
            for (index, name) in labels {
                PlaceButton { index, name, icon: Icon::Tag, shell, pages, badges }
            }
        }
    }
}

#[component]
fn PlaceButton(
    index: usize,
    name: String,
    icon: Icon,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
) -> Element {
    let on = shell.read().selected == index;
    let count = badges().get(index).copied().flatten();
    let state = use_hook(motion);
    let accepts = shell.read().places.get(index).is_some_and(drag::accepts);
    let look = state.map_or_else(Look::default, |state| look(&state, index, &name, accepts));
    // quire's place, in mailo's box that outlines it while a dragged row could land on it.
    rsx! {
        div { class: if look.can_drop { "place can-drop" } else { "place" },
            SidebarItem {
                kind: ItemKind::Place { icon },
                label: name.clone(),
                here: if on { Here::Current } else { Here::Elsewhere },
                // quire's count bumps itself whenever the number changes.
                count: count.map(count_of),
                presence: Presence::Present,
                preview: look.dest.then_some(ds::Preview::Destination),
                // The gulp plays for as long as the list's gulp timer runs for this place.
                pulse: if look.gulp {
                    PulseKey::rest(Anim::Gulp).fired()
                } else {
                    PulseKey::rest(Anim::Gulp)
                },
                onclick: move |()| {
                    shell.write().select(index);
                    pages.set(1);
                },
                onclose: None,
                drop: if look.target { DropState::Target } else { DropState::Idle },
                place: Some(PlaceId(name.clone())),
                onpointerenter: move |_| {
                    if accepts {
                        drag::over(Some(index), index);
                    }
                },
                onpointerleave: move |_| drag::over(None, index),
            }
        }
    }
}

/// How a place looks besides being there: where an op landed, where a hovered strip button
/// would send a row, and its part in a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Look {
    gulp: bool,
    dest: bool,
    /// Under a dragged row that it takes.
    target: bool,
    /// Takes the row being dragged, which is elsewhere.
    can_drop: bool,
}

fn look(state: &super::super::motion::Motion, index: usize, name: &str, accepts: bool) -> Look {
    let (target, can_drop) = match &*state.drag.read() {
        drag::Drag::Live { target, .. } if accepts => {
            let here = *target == Some(index);
            (here, !here)
        }
        _ => (false, false),
    };
    Look {
        gulp: state.gulp.read().as_deref() == Some(name),
        dest: *state.dest.read() == Some(name),
        target,
        can_drop,
    }
}

#[component]
pub(super) fn PinnedList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    space: Space,
    pins: Vec<u64>,
) -> Element {
    let driver = use_driver();
    // Each pin's box, as it mounts: its card is placed beside it. Not a signal: nothing
    // redraws for it.
    let boxes =
        use_hook(|| CopyValue::new(std::collections::HashMap::<usize, ds::MountedRef>::new()));
    rsx! {
        div { class: "s-h", "Pinned" }
        for (index, pin) in space.pins.iter().enumerate() {
            {
                let name = match pin {
                    Pinned::Person { name, .. } | Pinned::Search { name, .. } => name.clone(),
                };
                let query = match pin {
                    Pinned::Person { email, .. } => format!("from:{email}"),
                    Pinned::Search { query, .. } => query.clone(),
                };
                let avatar = AvatarFace {
                    initial: initial(&name),
                    size: AvatarSize::Size16,
                    tone: AvatarTone::Account(PersonSwatch::nth(index + 3).colour()),
                    shape: AvatarShape::Square,
                };
                let n = pins.get(index).copied().unwrap_or(0);
                rsx! {
                    // The item has no pointer hooks of its own, so the pin's card listens around it.
                    div {
                        key: "{name}",
                        "data-hc": "pin:{index}",
                        onmounted: move |event: MountedEvent| {
                            let mut boxes = boxes;
                            boxes.write().insert(index, ds::MountedRef(event.data()));
                        },
                        onpointerenter: move |_| {
                            over(driver, Hook::Pin(index), element(boxes.peek().get(&index).cloned()));
                        },
                        onpointerleave: move |_| out(driver),
                        SidebarItem {
                            kind: ItemKind::Pinned { avatar },
                            label: name.clone(),
                            here: Here::Elsewhere,
                            count: (n > 0).then(|| u32::try_from(n).unwrap_or(u32::MAX)),
                            presence: Presence::Present,
                            preview: None,
                            pulse: PulseKey::rest(Anim::Gulp),
                            onclick: move |()| {
                                shell.write().search = query.clone();
                                pages.set(1);
                            },
                            onclose: None,
                        }
                    }
                }
            }
        }
    }
}
