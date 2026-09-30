//! One place for every list of choices: quire's `Menu`, `CommandPalette` and `Popover`.
//!
//! mailo describes its rows as [`MenuItem`]s (a tile, a name, one line of help, and a shortcut, a
//! check or a remove on the right) and hands them to quire as one of three things:
//!
//! - [`Floating`]: an `NSMenu`, a short list of commands hung from the control that opened it.
//!   The Mac's menu has no help line and no filter, so a row's help is not drawn here; a list
//!   whose rows need it is a [`Picker`].
//! - [`Picker`]: a quire `CommandPalette` over the window, for a list a person narrows by typing
//!   (Move to, Import into): a field, the rows under their group headers, the query marked in
//!   each name, a help line under it.
//! - [`Checklist`]: a quire `Popover` of checkboxes, for a set a person toggles several of
//!   before dismissing it (Labels, the page's Properties).

mod state;

use super::common::classed;
use crate::search::match_list;
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::components::content::label::LabelRole;
use ds::components::content::text_runs::{RunTone, TextRun};
use ds::components::controls::button_model::Bezel;
use ds::components::controls::checkbox::Checkbox;
use ds::components::menus::item::item::MenuImage;
use ds::components::menus::palette::palette_group::{PaletteGroup, PaletteGroups, PaletteRow};
use ds::host::measure::{Anchor, MountedRef};
use ds::prelude::*;
pub(super) use state::{MenuItem, MenuKey, Piece, Right, Run, Tile, Tone, menu_key, pieces};

/// A list of commands drawn by quire's `Menu`: floating over the window, anchored to the element
/// that opened it, holding the keyboard while it is open. `anchor` is `None` only before that
/// element has mounted, as in a document with no renderer, where the menu sits at the corner.
///
/// `flow: Flow::Inline` draws the rows where the caller renders them (the sender card's
/// actions). A row's `help` is not drawn: the Mac's menu has no second line (design/27 5.1).
#[component]
pub(in crate::ui) fn Floating(
    #[props(default)] placement: MenuPlacement,
    anchor: Option<MountedRef>,
    /// The opener's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
    title: String,
    items: Vec<MenuItem>,
    /// A line under the rows that is not a choice: why there is nothing to choose.
    #[props(default)]
    note: Option<String>,
    /// Floating over the window, or drawn in the caller's flow.
    #[props(default)]
    flow: Flow,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let anchor = match placed {
        Some(rect) => Anchor::Rect(rect),
        None => anchor_at(anchor),
    };
    let mut choices = menu_items(&title, &items);
    if let Some(note) = note {
        choices.push(ds::components::menus::item::item::MenuItem::Info {
            title: note,
            detail: None,
        });
    }
    rsx! {
        Menu::<String> {
            placement,
            anchor,
            items: choices,
            flow,
            onpick: on_pick,
            onclose: on_close,
        }
    }
}

/// mailo's items as a menu's: the title and each new group become headers, a key is the value a
/// pick hands back, a check is the item's state mark (and then no image) and an icon tile is
/// its image.
pub(in crate::ui) fn menu_items(
    title: &str,
    items: &[MenuItem],
) -> Vec<ds::components::menus::item::item::MenuItem<String>> {
    use ds::components::menus::item::item::MenuItem as Line;
    let mut out = Vec::new();
    if !title.is_empty() {
        out.push(Line::Header(title.to_owned()));
    }
    let mut last: Option<&str> = None;
    for item in items {
        if let Some(group) = item.group.as_deref()
            && last != Some(group)
        {
            out.push(Line::Header(group.to_owned()));
            last = Some(group);
        }
        let mut line = Line::new(item.key.clone(), plain(item));
        // A choice among several is marked by its check and carries no picture: a menu of
        // states draws one mark per row, not a mark and an icon.
        line = match (&item.right, &item.tile) {
            (Right::Check(true), _) => line.with_check(Check::On),
            (Right::Check(false), _) => line.with_check(Check::Off),
            (Right::Shortcut(_) | Right::Remove(_) | Right::None, Tile::Icon(icon)) => {
                line.with_image(MenuImage::Icon(*icon))
            }
            (Right::Shortcut(_) | Right::Remove(_) | Right::None, _) => line,
        };
        out.push(line);
    }
    out
}

/// What an item is called, with its runs joined.
fn plain(item: &MenuItem) -> String {
    if item.title.is_empty() {
        item.name.clone()
    } else {
        item.title.iter().map(|run| run.text.as_str()).collect()
    }
}

/// Where a floating menu goes: against the element `at`, or the window's corner before that
/// element has mounted (a document with no renderer).
pub(in crate::ui) fn anchor_at(at: Option<MountedRef>) -> Anchor {
    at.map_or(Anchor::Point(Point::default()), Anchor::Mounted)
}

/// mailo's items as palette groups: one group per run of items that share a title, in the order
/// the items come; an item with no group joins the one before it, or an untitled first group.
/// A [`Right::Remove`] is the row's trailing x, which hands the item's key to `on_remove` and
/// picks nothing.
pub(in crate::ui) fn palette_groups(
    items: &[MenuItem],
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> PaletteGroups<String> {
    let mut groups: Vec<PaletteGroup<String>> = Vec::new();
    for item in items {
        let row = palette_row(item, avatar, on_remove);
        let title = item.group.as_deref();
        match (title, groups.last_mut()) {
            (Some(group), Some(last)) if last.title == group => push(last, row),
            (None, Some(last)) => push(last, row),
            (group, _) => groups.push(PaletteGroup::list(group.unwrap_or_default(), vec![row])),
        }
    }
    PaletteGroups(groups)
}

fn push(group: &mut PaletteGroup<String>, row: PaletteRow<String>) {
    if let ds::components::menus::palette::palette_group::GroupEntries::List(rows) =
        &mut group.entries
    {
        rows.push(row);
    }
}

fn palette_row(
    item: &MenuItem,
    avatar: AvatarSize,
    on_remove: Option<EventHandler<String>>,
) -> PaletteRow<String> {
    let title = if item.title.is_empty() {
        line_of(&[Run {
            text: item.name.clone(),
            marks: item.marks.clone(),
            tone: Tone::Plain,
        }])
    } else {
        line_of(&item.title)
    };
    let detail = if item.detail.is_empty() {
        item.help.clone().map(TextLine::from)
    } else {
        Some(line_of(&item.detail))
    };
    let accessory = match &item.right {
        Right::Shortcut(keys) => Accessory::Text(keys.clone()),
        Right::Check(true) => Accessory::Check(Check::On),
        Right::Check(false) | Right::Remove(_) | Right::None => Accessory::None,
    };
    let action = match (&item.right, on_remove) {
        (Right::Remove(label), Some(remove)) => {
            let key = item.key.clone();
            Some(RowAction::new(
                Icon::X,
                label.clone(),
                EventHandler::new(move |_| remove.call(key.clone())),
            ))
        }
        _ => None,
    };
    PaletteRow {
        detail,
        leading: leading(&item.tile, avatar),
        accessory,
        action,
        ..PaletteRow::new(item.key.clone(), title)
    }
}

/// Runs as quire's line: plain when nothing in them is marked or toned.
fn line_of(parts: &[Run]) -> TextLine {
    let plain = parts
        .iter()
        .all(|run| run.marks.is_empty() && run.tone == Tone::Plain);
    if plain {
        return TextLine::Plain(parts.iter().map(|run| run.text.as_str()).collect());
    }
    let mut runs = Vec::new();
    for run in parts {
        let tone = match run.tone {
            Tone::Plain => RunTone::Plain,
            Tone::Strong => RunTone::Strong,
            Tone::Faint => RunTone::Faint,
        };
        for piece in pieces(&run.text, &run.marks) {
            runs.push(match piece {
                Piece::Plain(text) => TextRun::new(text, tone),
                Piece::Mark(text) => TextRun::new(text, RunTone::Mark),
            });
        }
    }
    TextLine::Runs(runs)
}

fn leading(tile: &Tile, size: AvatarSize) -> RowLeading {
    match tile {
        Tile::Icon(icon) => RowLeading::Icon(*icon),
        Tile::Avatar { letter, color } => RowLeading::Avatar(AvatarFace {
            initial: *letter,
            size,
            tone: AvatarTone::Account(super::sidebar::hex_colour(color)),
            shape: AvatarShape::Round,
        }),
        Tile::Glyph(ch) => RowLeading::Text(ch.to_string()),
        Tile::Text(text) => RowLeading::Text((*text).to_owned()),
    }
}

/// A list a person narrows by typing: quire's `CommandPalette` over the window, its rows the
/// items ranked by mailo's fuzzy matcher, the query marked in each name.
///
/// A pick closes it and hands back the item's key. `empty` is what it says when nothing matches.
#[component]
pub(in crate::ui) fn Picker(
    label: String,
    placeholder: String,
    items: Vec<MenuItem>,
    empty: String,
    #[props(default)] on_remove: Option<EventHandler<String>>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let mut query = use_signal(String::new);
    let shown = narrowed(&items, &query());
    let groups = palette_groups(&shown, AvatarSize::Size22, on_remove);
    rsx! {
        CommandPalette::<String> {
            label,
            placeholder,
            query: query(),
            tokens: Vec::new(),
            groups,
            empty,
            oninput: move |text: String| query.set(text),
            onpick: on_pick,
            onclose: on_close,
        }
    }
}

/// The items a query keeps, best match first with the matched characters marked; every item, in
/// order, while the query is empty.
pub(in crate::ui) fn narrowed(items: &[MenuItem], query: &str) -> Vec<MenuItem> {
    if query.trim().is_empty() {
        return items.to_vec();
    }
    let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    match_list(query, &names)
        .into_iter()
        .filter_map(|hit| {
            items.get(hit.index).map(|item| MenuItem {
                marks: hit.indices,
                ..item.clone()
            })
        })
        .collect()
}

/// A set a person toggles several of: a quire `Popover` of `Checkbox`es under the control that
/// opened it. It stays up as each is toggled and closes on Escape or a click outside. `on_pick`
/// hears the key of the one toggled. An item with no check (`Right::Check`) is an action, drawn
/// as an inline button ("Create a label"). With a `placeholder` the popover opens with a search
/// field whose text `on_query` hears; the caller narrows `items` by it.
#[component]
pub(in crate::ui) fn Checklist(
    anchor: Option<MountedRef>,
    #[props(default)] placed: Option<Rect>,
    title: String,
    items: Vec<MenuItem>,
    #[props(default)] placeholder: Option<String>,
    #[props(default)] query: String,
    #[props(default)] on_query: Option<EventHandler<String>>,
    #[props(default)] note: Option<String>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let anchor = match placed {
        Some(rect) => Anchor::Rect(rect),
        None => anchor_at(anchor),
    };
    rsx! {
        Popover {
            anchor,
            placement: ds::base::geometry::placement::Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: ds::components::overlays::popover::Arrow::None,
            onclose: on_close,
            div { class: "checklist",
                if !title.is_empty() {
                    Label { text: title, role: LabelRole::Secondary }
                }
                if let Some(placeholder) = placeholder {
                    TextField {
                        label: placeholder.clone(),
                        placeholder,
                        value: query,
                        kind: FieldKind::Search,
                        focus: FieldFocus::OnMount,
                        common: classed("checklist-filter"),
                        oninput: move |text: String| {
                            if let Some(on_query) = on_query {
                                on_query.call(text);
                            }
                        },
                    }
                }
                for item in items {
                    match item.right {
                        Right::Check(on) => rsx! {
                            Checkbox {
                                key: "{item.key}",
                                label: plain(&item),
                                value: if on { Check::On } else { Check::Off },
                                onchange: {
                                    let key = item.key.clone();
                                    move |_| on_pick.call(key.clone())
                                },
                            }
                        },
                        _ => rsx! {
                            Button {
                                key: "{item.key}",
                                bezel: Bezel::Inline,
                                label: plain(&item),
                                onclick: {
                                    let key = item.key.clone();
                                    move |_| on_pick.call(key.clone())
                                },
                            }
                        },
                    }
                }
                if let Some(note) = note {
                    Label { text: note, role: LabelRole::Tertiary }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
