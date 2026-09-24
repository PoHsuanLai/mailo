//! One menu for every list of choices.
//!
//! A title row, then items. Each item has a tile, a name, one line of help, and a shortcut or a
//! check on the right. The arrow keys, Enter and Esc live on [`MenuState`], so the table in the
//! tests never needs a document.

mod state;

use super::field::{Field, FieldKind};
use dioxus::prelude::*;
use ds::{
    Anchor, Availability, AvatarFace, AvatarShape, AvatarSize, AvatarTone, Check, Filter, Glyph,
    Icon, MenuEntry, MenuKind, MountedRef, Point, Trail,
};
pub(super) use state::{
    MenuEvent, MenuItem, MenuKey, MenuState, Piece, Right, Run, Shown, Tile, Tone, menu_key, pieces,
};

/// A header or an item, in the order the menu paints them.
enum Row {
    Header(String),
    Item(usize),
}

fn rows(shown: &[Shown<'_>]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut last: Option<&str> = None;
    for (index, shown) in shown.iter().enumerate() {
        if let Some(group) = shown.item.group.as_deref()
            && last != Some(group)
        {
            out.push(Row::Header(group.to_owned()));
            last = Some(group);
        }
        out.push(Row::Item(index));
    }
    out
}

/// The shared menu. `active` set means the parent owns the cursor (the command menu's field
/// sits outside this element). Otherwise the menu owns its [`MenuState`].
#[component]
pub(super) fn Menu(
    title: String,
    items: Vec<MenuItem>,
    filterable: bool,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
    on_query: EventHandler<String>,
    slim: bool,
    active: Option<usize>,
    /// What a [`Right::Remove`] × does with its item's key. Only menus that offer one pass it.
    on_remove: Option<EventHandler<String>>,
) -> Element {
    let mut state = use_signal(|| MenuState::new(filterable));
    let mut held = use_signal(|| items.clone());
    if held.read().as_slice() != items.as_slice() {
        held.set(items.clone());
    }
    let live = held.read().clone();
    let shown = state.read().shown(&live);
    let cursor = active.unwrap_or_else(|| state.read().active().min(shown.len().saturating_sub(1)));
    let class = if slim { "fmenu slim" } else { "fmenu" };
    let owned = active.is_none();
    let painted = rows(&shown);
    let filter_value = state.read().query().to_owned();
    rsx! {
        div {
            class: "{class}",
            role: "listbox",
            tabindex: "0",
            onmousedown: move |event| {
                event.prevent_default();
                event.stop_propagation();
            },
            onkeydown: move |event| {
                if !owned {
                    return;
                }
                let Some(key) = menu_key(&event.key().to_string()) else {
                    return;
                };
                // Letters belong to the field. Handling them here as well would type each one twice.
                if matches!(key, MenuKey::Character(_) | MenuKey::Backspace) {
                    return;
                }
                event.stop_propagation();
                let current: Vec<MenuItem> = state
                    .read()
                    .shown(&held.read())
                    .into_iter()
                    .map(|shown| shown.item.clone())
                    .collect();
                match state.write().on_key(key, &current) {
                    MenuEvent::Pick(key) => on_pick.call(key),
                    MenuEvent::Close => on_close.call(()),
                    MenuEvent::Moved | MenuEvent::Typed | MenuEvent::Ignored => {}
                }
            },
            if !title.is_empty() {
                div { class: "g", "{title}" }
            }
            if filterable {
                Field {
                    kind: FieldKind::Inline,
                    value: filter_value,
                    placeholder: "Filter".to_owned(),
                    extra: None,
                    on_input: move |value: String| {
                        state.write().set_query(value.clone());
                        on_query.call(value);
                    },
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
            }
            if shown.is_empty() {
                div { class: "none", "Nothing matches." }
            }
            for (n, row) in painted.into_iter().enumerate() {
                match row {
                    Row::Header(group) => rsx! { div { key: "g-{n}-{group}", class: "g", "{group}" } },
                    Row::Item(index) => {
                        let shown = &shown[index];
                        let key = shown.item.key.clone();
                        let indices = if shown.indices.is_empty() {
                            shown.item.marks.clone()
                        } else {
                            shown.indices.clone()
                        };
                        let title = if shown.item.title.is_empty() {
                            vec![Run {
                                text: shown.item.name.clone(),
                                marks: indices,
                                tone: Tone::Plain,
                            }]
                        } else {
                            shown.item.title.clone()
                        };
                        let detail = if shown.item.detail.is_empty() {
                            shown
                                .item
                                .help
                                .clone()
                                .map(|text| Run {
                                    text,
                                    marks: Vec::new(),
                                    tone: Tone::Plain,
                                })
                                .into_iter()
                                .collect()
                        } else {
                            shown.item.detail.clone()
                        };
                        let selected = index == cursor;
                        let checked = matches!(shown.item.right, Right::Check(true));
                        let removed = key.clone();
                        rsx! {
                            div {
                                key: "{key}",
                                class: "it",
                                role: "option",
                                aria_selected: if selected { "true" } else { "false" },
                                aria_checked: if checked { "true" } else { "false" },
                                onclick: move |event| {
                                    event.stop_propagation();
                                    on_pick.call(key.clone());
                                },
                                {tile(&shown.item.tile)}
                                span {
                                    b { {runs(&title)} }
                                    if !detail.is_empty() {
                                        small { {runs(&detail)} }
                                    }
                                }
                                span { class: "sc",
                                    match &shown.item.right {
                                        Right::Shortcut(shortcut) => rsx! { "{shortcut}" },
                                        Right::Check(true) => rsx! { Glyph { icon: Icon::Check, size: ds::IconSize::Compact } },
                                        Right::Remove(label) => rsx! {
                                            button {
                                                class: "rm",
                                                r#type: "button",
                                                aria_label: "{label}",
                                                title: "{label}",
                                                onclick: move |event| {
                                                    event.stop_propagation();
                                                    if let Some(remove) = on_remove {
                                                        remove.call(removed.clone());
                                                    }
                                                },
                                                Glyph { icon: Icon::X, size: ds::IconSize::Small }
                                            }
                                        },
                                        Right::Check(false) | Right::None => rsx! { "" },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A list of choices drawn by quire's `Menu`: floating over the window, anchored to the element
/// that opened it, holding the keyboard while it is open. `anchor` is `None` only before that
/// element has mounted, as in a document with no renderer, where the menu sits at the corner.
///
/// Every menu that owns its own cursor is one of these. [`Menu`] is left for the menus quire's
/// cannot be yet: a cursor the field beside it drives, a typed filter the caller hears, or a
/// row's own remove button.
#[component]
pub(in crate::ui) fn Floating(
    kind: MenuKind,
    anchor: Option<MountedRef>,
    title: String,
    items: Vec<MenuItem>,
    /// Typing narrows the rows, as quire's ranker marks them.
    #[props(default)]
    filter: Filter,
    /// A line under the rows that is not a choice: why there is nothing to choose.
    #[props(default)]
    note: Option<String>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let anchor = anchor.map_or(Anchor::Point(Point::default()), Anchor::Mounted);
    let avatar = match kind {
        MenuKind::Rich => AvatarSize::Size34,
        MenuKind::Slim | MenuKind::Dropdown | MenuKind::Context => AvatarSize::Size20,
    };
    let mut entries = entries(&title, &items, avatar);
    if let Some(note) = note {
        entries.push(MenuEntry::Info {
            title: note,
            detail: None,
        });
    }
    rsx! {
        ds::Menu::<String> {
            kind,
            anchor,
            entries,
            filter,
            onpick: on_pick,
            onclose: on_close,
        }
    }
}

/// mailo's items as quire's entries: the title and each new group become headers, a key is
/// the value a pick hands back, and a check is the item's check mark.
pub(in crate::ui) fn entries(
    title: &str,
    items: &[MenuItem],
    avatar: AvatarSize,
) -> Vec<MenuEntry<String>> {
    let mut out = Vec::new();
    if !title.is_empty() {
        out.push(MenuEntry::Header(title.to_owned()));
    }
    let mut last: Option<&str> = None;
    for item in items {
        if let Some(group) = item.group.as_deref()
            && last != Some(group)
        {
            out.push(MenuEntry::Header(group.to_owned()));
            last = Some(group);
        }
        let joined = |parts: &[Run]| {
            parts
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        };
        let title = if item.title.is_empty() {
            item.name.clone()
        } else {
            joined(&item.title)
        };
        let detail = if item.detail.is_empty() {
            item.help.clone()
        } else {
            Some(joined(&item.detail))
        };
        let (trail, check) = match &item.right {
            Right::Shortcut(keys) => (Trail::Note(keys.clone()), None),
            Right::Check(true) => (Trail::None, Some(Check::Checked)),
            Right::Check(false) => (Trail::None, Some(Check::Unchecked)),
            Right::Remove(_) | Right::None => (Trail::None, None),
        };
        out.push(MenuEntry::Item {
            value: item.key.clone(),
            title,
            detail,
            tile: Some(quire_tile(&item.tile, avatar)),
            trail,
            check,
            availability: Availability::Enabled,
        });
    }
    out
}

/// quire's entries for a floating menu: its title and each new group as headers, then mailo's
/// items as [`quire_rows`].
pub(in crate::ui) fn quire_entries(
    title: &str,
    items: &[MenuItem],
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> Vec<MenuEntry<String>> {
    let mut out = Vec::new();
    if !title.is_empty() {
        out.push(MenuEntry::Header(title.to_owned()));
    }
    let mut last: Option<&str> = None;
    for item in items {
        if let Some(group) = item.group.as_deref()
            && last != Some(group)
        {
            out.push(MenuEntry::Header(group.to_owned()));
            last = Some(group);
        }
        out.push(quire_row(item, avatar, on_remove));
    }
    out
}

/// Where a floating menu goes: against the element `at`, or the window's corner before that
/// element has mounted (a document with no renderer).
pub(in crate::ui) fn anchor_at(at: Option<MountedRef>) -> Anchor {
    at.map_or(Anchor::Point(Point::default()), Anchor::Mounted)
}

/// mailo's items as quire's rows, each carrying its own runs: a marked piece is a `Mark` run, a
/// strong or faint run keeps its tone, and a name with no marks and no tone stays plain, so the
/// menu's own filter can still mark it. A [`Right::Remove`] is the row's trailing ×, which hands
/// the item's key to `on_remove` and picks nothing.
pub(in crate::ui) fn quire_rows(
    items: &[MenuItem],
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> Vec<MenuEntry<String>> {
    items
        .iter()
        .map(|item| quire_row(item, avatar, on_remove))
        .collect()
}

/// [`quire_rows`] under their group titles, as quire's palette takes them: one group per run of
/// items that share a title, in the order the items come; an item with no group joins the one
/// before it, or an untitled first group.
pub(in crate::ui) fn quire_groups(
    items: &[MenuItem],
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> Vec<(String, Vec<MenuEntry<String>>)> {
    let mut groups: Vec<(String, Vec<MenuEntry<String>>)> = Vec::new();
    for item in items {
        let row = quire_row(item, avatar, on_remove);
        match (item.group.as_deref(), groups.last_mut()) {
            (Some(group), Some((title, rows))) if title == group => rows.push(row),
            (None, Some((_, rows))) => rows.push(row),
            (group, _) => groups.push((group.unwrap_or_default().to_owned(), vec![row])),
        }
    }
    groups
}

fn quire_row(
    item: &MenuItem,
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> MenuEntry<String> {
    let title = if item.title.is_empty() {
        text_of(&[Run {
            text: item.name.clone(),
            marks: item.marks.clone(),
            tone: Tone::Plain,
        }])
    } else {
        text_of(&item.title)
    };
    let detail = if item.detail.is_empty() {
        item.help.clone().map(ds::Text::from)
    } else {
        Some(text_of(&item.detail))
    };
    let (trail, check) = match &item.right {
        Right::Shortcut(keys) => (Trail::Note(keys.clone()), None),
        Right::Check(true) => (Trail::None, Some(Check::Checked)),
        Right::Check(false) => (Trail::None, Some(Check::Unchecked)),
        Right::Remove(_) | Right::None => (Trail::None, None),
    };
    let trailing = match (&item.right, on_remove) {
        (Right::Remove(label), Some(remove)) => {
            let key = item.key.clone();
            Some(ds::RowAction {
                icon: Icon::X,
                label: label.clone(),
                on_press: EventHandler::new(move |_| remove.call(key.clone())),
            })
        }
        _ => None,
    };
    MenuEntry::Row(ds::MenuRow {
        tile: Some(quire_tile(&item.tile, avatar)),
        detail,
        trail,
        check,
        trailing,
        ..ds::MenuRow::new(item.key.clone(), title)
    })
}

/// Runs as quire's text: plain when nothing in them is marked or toned.
fn text_of(parts: &[Run]) -> ds::Text {
    let plain = parts
        .iter()
        .all(|run| run.marks.is_empty() && run.tone == Tone::Plain);
    if plain {
        return ds::Text::Plain(parts.iter().map(|run| run.text.as_str()).collect());
    }
    let mut runs = Vec::new();
    for run in parts {
        let tone = match run.tone {
            Tone::Plain => ds::RunTone::Plain,
            Tone::Strong => ds::RunTone::Strong,
            Tone::Faint => ds::RunTone::Faint,
        };
        for piece in pieces(&run.text, &run.marks) {
            runs.push(match piece {
                Piece::Plain(text) => ds::Run::new(text, tone),
                Piece::Mark(text) => ds::Run::new(text, ds::RunTone::Mark),
            });
        }
    }
    ds::Text::Runs(runs)
}

fn quire_tile(tile: &Tile, size: AvatarSize) -> ds::Tile {
    match tile {
        Tile::Icon(icon) => ds::Tile::Icon(*icon),
        Tile::Avatar { letter, color } => ds::Tile::Avatar(AvatarFace {
            initial: *letter,
            size,
            tone: AvatarTone::Account(super::sidebar::hex_colour(color)),
            shape: AvatarShape::Round,
        }),
        Tile::Glyph(ch) => ds::Tile::Text(ch.to_string()),
        Tile::Text(text) => ds::Tile::Text((*text).to_owned()),
    }
}

fn runs(parts: &[Run]) -> Element {
    rsx! {
        for (index, run) in parts.iter().enumerate() {
            span { key: "{index}", class: tone_class(run.tone),
                for (piece_index, piece) in pieces(&run.text, &run.marks).into_iter().enumerate() {
                    match piece {
                        Piece::Plain(text) => rsx! { span { key: "p{piece_index}", "{text}" } },
                        Piece::Mark(text) => rsx! { mark { key: "m{piece_index}", "{text}" } },
                    }
                }
            }
        }
    }
}

fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Plain => "",
        Tone::Strong => "who",
        Tone::Faint => "addr",
    }
}

fn tile(tile: &Tile) -> Element {
    match tile {
        Tile::Icon(icon) => {
            rsx! { span { class: "tile", Glyph { icon: *icon, size: ds::IconSize::Tile } } }
        }
        Tile::Avatar { letter, color } => rsx! {
            span {
                class: "tile round",
                style: "background:{color};border-color:transparent",
                "{letter}"
            }
        },
        Tile::Glyph(ch) => rsx! { span { class: "tile", "{ch}" } },
        Tile::Text(text) => rsx! { span { class: "tile", "{text}" } },
    }
}

#[cfg(test)]
mod tests;
