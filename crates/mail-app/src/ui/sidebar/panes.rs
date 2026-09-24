//! Account tiles, places, labels and pinned people.

use super::super::data::account_rows;
use super::super::hover::{Hook, corner, hover};
use super::super::motion::{drag, motion};
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::provider::{Provider, provider};
use crate::query::{self};
use crate::space::{self, Pinned, Scope, Space};
use crate::view::{Shell, Source, folder_of, is_label_place};
use dioxus::prelude::*;
use ds::{Glyph, Icon};
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

fn initial(text: &str) -> String {
    let mut out = String::new();
    if let Some(ch) = text.chars().next() {
        out.extend(ch.to_uppercase());
    }
    if out.is_empty() { "?".to_owned() } else { out }
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
    rsx! {
        div { class: "pins", role: "group", aria_label: "Accounts in this Space",
            if several {
                button {
                    class: "pin acct",
                    aria_pressed: if shell.read().account.is_none() { "true" } else { "false" },
                    aria_label: "All accounts",
                    onclick: move |_| {
                        shell.write().account = None;
                        pages.set(1);
                    },
                    span { class: "av all", Glyph { icon: Icon::Inbox } }
                    if counted.all > 0 {
                        span { class: "n", "{counted.all}" }
                    }
                }
            }
            for (index, row) in counted.rows.iter().enumerate() {
                {
                    let id = row.0;
                    let address = row.1.clone();
                    let n = row.2;
                    let via = row.3;
                    let on = !several || shell.read().account == Some(id);
                    let color = space::avatar_color(&space, id, index);
                    let letter = initial(&address);
                    rsx! {
                        button {
                            key: "{id}",
                            class: "pin acct",
                            aria_pressed: if on { "true" } else { "false" },
                            aria_label: "{address}",
                            onclick: move |_| {
                                shell.write().account = Some(id);
                                pages.set(1);
                            },
                            span { class: "av", style: "background:{color}", "{letter}" }
                            if let Some(via) = via {
                                ProvChip { provider: via, marks: shell.read().appearance.marks, place: ChipPlace::Tile }
                            }
                            if n > 0 {
                                span { class: "n", "{n}" }
                            }
                        }
                    }
                }
            }
            button {
                class: "pin acct acct-add",
                r#type: "button",
                title: "Add account…",
                aria_label: "Add account",
                onclick: move |_| super::super::add_account::open(shell),
                span { class: "av", Glyph { icon: Icon::Plus } }
            }
        }
    }
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
    // The count this place showed last, so a change can bump. Kept apart from the badge memo:
    // a bump is "the number changed", and only an effect sees both numbers.
    let mut shown = use_signal(|| count);
    let mut bumping = use_signal(|| false);
    use_effect(move || {
        let now = badges().get(index).copied().flatten();
        if now != *shown.peek() {
            let had = shown.peek().is_some();
            shown.set(now);
            bumping.set(had && now.is_some());
        }
    });
    let state = use_hook(motion);
    let accepts = shell.read().places.get(index).is_some_and(drag::accepts);
    let place = name.clone();
    let class = state.map_or_else(
        || "item".to_owned(),
        |state| place_class(&state, index, &place, accepts),
    );
    rsx! {
        button {
            class: "{class}",
            "data-place": "{name}",
            aria_current: if on { "true" } else { "false" },
            onclick: move |_| {
                shell.write().select(index);
                pages.set(1);
            },
            onpointerenter: move |_| {
                if accepts {
                    drag::over(Some(index), index);
                }
            },
            onpointerleave: move |_| drag::over(None, index),
            onanimationend: move |event: Event<AnimationData>| {
                if event.animation_name() == "gulp"
                    && let Some(mut state) = motion()
                {
                    state.gulp.set(None);
                }
            },
            Glyph { icon }
            span { "{name}" }
            if let Some(count) = count {
                span {
                    class: if bumping() { "count bump" } else { "count" },
                    onanimationend: move |event: Event<AnimationData>| {
                        event.stop_propagation();
                        bumping.set(false);
                    },
                    "{count}"
                }
            }
        }
    }
}

/// A place's classes: where an op landed, where a hovered button would send a row, and
/// whether it takes the row being dragged.
fn place_class(
    state: &super::super::motion::Motion,
    index: usize,
    name: &str,
    accepts: bool,
) -> String {
    let mut class = String::from("item");
    if state.gulp.read().as_deref() == Some(name) {
        class.push_str(" gulp");
    }
    if *state.dest.read() == Some(name) {
        class.push_str(" dest");
    }
    if let drag::Drag::Live { target, .. } = &*state.drag.read() {
        if *target == Some(index) && accepts {
            class.push_str(" is-drop-target");
        } else if accepts {
            class.push_str(" can-drop");
        }
    }
    class
}

#[component]
pub(super) fn PinnedList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    space: Space,
    pins: Vec<u64>,
) -> Element {
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
                let letter = initial(&name);
                let color = space::AVATAR[(index + 3) % space::AVATAR.len()];
                let n = pins.get(index).copied().unwrap_or(0);
                rsx! {
                    button {
                        key: "{name}",
                        class: "item pinned",
                        "data-hc": "pin:{index}",
                        onpointerenter: move |event| {
                            if let Some(hover) = hover() {
                                hover.enter(Hook::Pin(index), corner(&event));
                            }
                        },
                        onpointerleave: move |_| {
                            if let Some(hover) = hover() {
                                hover.leave();
                            }
                        },
                        onclick: move |_| {
                            shell.write().search = query.clone();
                            pages.set(1);
                        },
                        span { class: "fav", style: "background:{color}", "{letter}" }
                        span { "{name}" }
                        if n > 0 {
                            span { class: "count", "{n}" }
                        }
                    }
                }
            }
        }
    }
}
