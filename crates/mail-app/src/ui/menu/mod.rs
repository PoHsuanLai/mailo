//! One menu for every list of choices: quire's `Menu`.
//!
//! mailo describes its rows as [`MenuItem`]s (a tile, a name, one line of help, and a shortcut, a
//! check or a remove on the right) and hands them to quire as entries, rows or groups. Every
//! menu is quire's: [`Floating`] for one anchored to what opened it, a checklist that stays open,
//! or rows drawn inside a card.

mod state;

use dioxus::prelude::*;
use ds::{
    Anchor, Availability, AvatarFace, AvatarShape, AvatarSize, AvatarTone, Check, Cursor, Filter,
    Flow, Icon, MenuEntry, MenuKind, MountedRef, PickDismiss, Point, Trail,
};
pub(super) use state::{MenuItem, MenuKey, Piece, Right, Run, Tile, Tone, menu_key, pieces};

/// A list of choices drawn by quire's `Menu`: floating over the window, anchored to the element
/// that opened it, holding the keyboard while it is open. `anchor` is `None` only before that
/// element has mounted, as in a document with no renderer, where the menu sits at the corner.
///
/// `dismiss: PickDismiss::Stay` keeps a toggle checklist open as each row is picked (Labels,
/// the page's Properties); `flow: Flow::Inline` draws the rows where the caller renders them
/// (the sender card's actions); `on_query` hears the typed filter (a "Create …" row).
#[component]
pub(in crate::ui) fn Floating(
    kind: MenuKind,
    anchor: Option<MountedRef>,
    /// The opener's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<ds::Rect>,
    title: String,
    items: Vec<MenuItem>,
    /// Typing narrows the rows, as quire's ranker marks them.
    #[props(default)]
    filter: Filter,
    /// A line under the rows that is not a choice: why there is nothing to choose.
    #[props(default)]
    note: Option<String>,
    /// Whether a pick closes the menu.
    #[props(default)]
    dismiss: PickDismiss,
    /// Floating over the window, or drawn in the caller's flow.
    #[props(default)]
    flow: Flow,
    /// Whose highlight the rows show.
    #[props(default)]
    active: Cursor,
    /// Hears the typed filter's text as it changes.
    #[props(default)]
    on_query: Option<EventHandler<String>>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let anchor = match placed {
        Some(rect) => Anchor::Rect(rect),
        None => anchor_at(anchor),
    };
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
            dismiss,
            flow,
            active,
            onquery: on_query,
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

#[cfg(test)]
mod tests;
