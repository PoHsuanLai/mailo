//! The rows of the page's menus: `/`, Turn into, `@`, people, and the object menu. Each is a
//! [`MenuItem`] for the one shared [`super::super::menu::Menu`].

use super::super::menu::{MenuItem, Right, Tile};
use super::page::Page;
use crate::editor::{Action, Item, ParaKind, Person, filter, turn_into};
use ds::Icon;

fn tile(item: &Item) -> Tile {
    match item.action {
        Action::Turn(ParaKind::Paragraph) => Tile::Text("Aa"),
        Action::Turn(ParaKind::Heading(level)) => Tile::Text(match level.number() {
            1 => "H1",
            2 => "H2",
            _ => "H3",
        }),
        Action::Turn(ParaKind::Bullet) => Tile::Glyph('•'),
        Action::Turn(ParaKind::Numbered) => Tile::Text("1."),
        Action::Turn(ParaKind::Todo(_)) => Tile::Glyph('☐'),
        Action::Turn(ParaKind::Quote) => Tile::Glyph('❝'),
        Action::Turn(ParaKind::Code) => Tile::Text("{}"),
        Action::Divider => Tile::Glyph('—'),
        Action::Image | Action::Attachment => Tile::Icon(Icon::Paperclip),
        Action::Table => Tile::Icon(Icon::Columns),
        Action::Signature => Tile::Text("--"),
        Action::Snippet => Tile::Icon(Icon::Pen),
        Action::Date => Tile::Icon(Icon::Clock),
    }
}

fn row(item: &Item, grouped: bool, right: Right) -> MenuItem {
    let group = grouped.then(|| match item.action {
        Action::Turn(_) => "Turn this line into".to_owned(),
        _ => "Insert".to_owned(),
    });
    MenuItem {
        key: item.name.to_owned(),
        tile: tile(item),
        name: item.name.to_owned(),
        help: Some(item.help.to_owned()),
        right,
        group,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// The `/` menu for `query`, filtered by `editor::slash`.
pub(in crate::ui) fn slash_items(query: &str) -> Vec<MenuItem> {
    let grouped = query.trim().is_empty();
    filter(query)
        .into_iter()
        .map(|item| row(item, grouped, Right::Shortcut(item.markdown.to_owned())))
        .collect()
}

/// Turn into, with the current kind checked.
pub(in crate::ui) fn turn_items(current: Option<ParaKind>) -> Vec<MenuItem> {
    turn_into()
        .into_iter()
        .map(|item| {
            let on = matches!(item.action, Action::Turn(kind) if Some(kind) == current);
            row(item, false, Right::Check(on))
        })
        .collect()
}

/// The book's suggestions for the `@` query, as [`super::float::suggest_mention`] left them.
pub(in crate::ui) fn mention_items(page: &Page) -> Vec<MenuItem> {
    people_rows(page.people.iter().collect())
}

/// Menu rows for people: an avatar, the name, the address.
pub(in crate::ui) fn people_rows(people: Vec<&Person>) -> Vec<MenuItem> {
    people
        .into_iter()
        .map(|person| MenuItem {
            key: person.address.clone(),
            tile: Tile::Avatar {
                letter: person
                    .name
                    .chars()
                    .next()
                    .and_then(|ch| ch.to_uppercase().next())
                    .unwrap_or('?'),
                color: crate::space::AVATAR[hue(&person.address) % crate::space::AVATAR.len()]
                    .to_owned(),
            },
            name: person.name.clone(),
            help: Some(person.address.clone()),
            right: Right::None,
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect()
}

/// A stable index for an avatar colour.
pub(in crate::ui) fn hue(address: &str) -> usize {
    address.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as usize)
    })
}

/// The ⋮⋮ menu for an object.
pub(in crate::ui) fn object_items() -> Vec<MenuItem> {
    [
        ("up", "Move up", Icon::Corner, ""),
        ("down", "Move down", Icon::Corner, ""),
        ("dup", "Duplicate", Icon::Square, ""),
        ("del", "Delete", Icon::Trash, "Del"),
    ]
    .into_iter()
    .map(|(key, name, icon, shortcut)| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: None,
        right: Right::Shortcut(shortcut.to_owned()),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    })
    .collect()
}
