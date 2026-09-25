//! mailo's menu rows as data, before they become quire's entries: the items, the keys a field
//! beside a menu hears, and the marks a query leaves in a name.

use ds::Icon;

/// What the tile on the left is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Tile {
    Icon(Icon),
    Avatar {
        letter: char,
        color: String,
    },
    Glyph(char),
    /// A short mark of more than one character, like `H1` or `1.`.
    Text(&'static str),
}

/// The right-hand side of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Right {
    Shortcut(String),
    Check(bool),
    /// A trailing ×, labelled with these words, that hands the item's key to `on_remove`.
    Remove(String),
    None,
}

/// How a run of text sits on the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Tone {
    /// The row's own type.
    Plain,
    /// Ink, heavier: a sender's name on a mail line.
    Strong,
    /// The data face, faint: an address.
    Faint,
}

/// A piece of a name or a help line, with the characters the query marked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Run {
    pub text: String,
    pub marks: Vec<u32>,
    pub tone: Tone,
}

/// One choice.
///
/// `name` is what the filter compares. `title` and `detail`, when set, are what is drawn:
/// a person's name and address are two runs, and a mail line is the sender then the snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct MenuItem {
    pub key: String,
    pub tile: Tile,
    pub name: String,
    pub help: Option<String>,
    pub right: Right,
    pub group: Option<String>,
    /// Character indices already marked in `name`, when `title` is empty.
    pub marks: Vec<u32>,
    pub title: Vec<Run>,
    pub detail: Vec<Run>,
}

/// A DOM key name, as `KeyboardEvent.key` reports it.
pub(in crate::ui) fn menu_key(name: &str) -> Option<MenuKey> {
    match name {
        "ArrowDown" => Some(MenuKey::Down),
        "ArrowUp" => Some(MenuKey::Up),
        "Enter" => Some(MenuKey::Enter),
        "Escape" => Some(MenuKey::Escape),
        "Backspace" => Some(MenuKey::Backspace),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if !ch.is_control() => Some(MenuKey::Character(ch)),
                _ => None,
            }
        }
    }
}

/// A key the menu understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum MenuKey {
    Up,
    Down,
    Enter,
    Escape,
    Character(char),
    Backspace,
}

/// A run of text, with the matched characters split out so they can be `<mark>` nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Piece {
    Plain(String),
    Mark(String),
}

/// Split `text` into plain and marked runs from character indices.
pub(in crate::ui) fn pieces(text: &str, indices: &[u32]) -> Vec<Piece> {
    let marked: std::collections::BTreeSet<usize> =
        indices.iter().map(|index| *index as usize).collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut on = false;
    for (index, ch) in text.chars().enumerate() {
        let next = marked.contains(&index);
        if next != on && !buf.is_empty() {
            out.push(if on {
                Piece::Mark(std::mem::take(&mut buf))
            } else {
                Piece::Plain(std::mem::take(&mut buf))
            });
        }
        on = next;
        buf.push(ch);
    }
    if !buf.is_empty() {
        out.push(if on {
            Piece::Mark(buf)
        } else {
            Piece::Plain(buf)
        });
    }
    out
}
