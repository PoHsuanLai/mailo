//! The pure half of [`super::Menu`]: the items, the keys, the cursor, the filter and the marks.
//! Nothing here needs a document, so the key table in `tests.rs` runs without one.

use super::super::icon::Icon;

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

/// What that key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum MenuEvent {
    Moved,
    Pick(String),
    Close,
    Typed,
    Ignored,
}

/// A run of text, with the matched characters split out so they can be `<mark>` nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Piece {
    Plain(String),
    Mark(String),
}

/// The cursor and the filter, with no document attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct MenuState {
    query: String,
    active: usize,
    filterable: bool,
}

/// One item that survived the filter, and the character indices the query marked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Shown<'a> {
    pub item: &'a MenuItem,
    pub indices: Vec<u32>,
}

impl MenuState {
    /// A menu that filters as the user types, or one that does not.
    pub(in crate::ui) fn new(filterable: bool) -> Self {
        Self {
            query: String::new(),
            active: 0,
            filterable,
        }
    }

    /// What is typed into the filter.
    pub(in crate::ui) fn query(&self) -> &str {
        &self.query
    }

    /// The highlighted row, among the items [`Self::shown`] returns.
    pub(in crate::ui) fn active(&self) -> usize {
        self.active
    }

    /// Replace the filter. The cursor goes back to the first row.
    pub(in crate::ui) fn set_query(&mut self, query: String) {
        if query != self.query {
            self.query = query;
            self.active = 0;
        }
    }

    /// Put the cursor back on the first row. The command menu does this on every new query.
    pub(in crate::ui) fn restart(&mut self) {
        self.active = 0;
    }

    /// Move, pick, close, or type. `shown` is the list the cursor is walking.
    pub(in crate::ui) fn on_key(&mut self, key: MenuKey, shown: &[MenuItem]) -> MenuEvent {
        if self.active >= shown.len() {
            self.active = 0;
        }
        match key {
            MenuKey::Down => self.move_by(1, shown.len()),
            MenuKey::Up => self.move_by(shown.len().saturating_sub(1), shown.len()),
            MenuKey::Enter => match shown.get(self.active) {
                Some(item) => MenuEvent::Pick(item.key.clone()),
                None => MenuEvent::Ignored,
            },
            MenuKey::Escape => MenuEvent::Close,
            MenuKey::Character(ch) if self.filterable && !ch.is_control() => {
                self.query.push(ch);
                self.active = 0;
                MenuEvent::Typed
            }
            MenuKey::Backspace if self.filterable && !self.query.is_empty() => {
                self.query.pop();
                self.active = 0;
                MenuEvent::Typed
            }
            MenuKey::Character(_) | MenuKey::Backspace => MenuEvent::Ignored,
        }
    }

    /// The items the query keeps, in their original order, with the matched character indices.
    ///
    /// Order is kept so a trailing "Create" row stays last. An empty query keeps every item and
    /// marks nothing. A query that matches nothing returns an empty list.
    pub(in crate::ui) fn shown<'a>(&self, items: &'a [MenuItem]) -> Vec<Shown<'a>> {
        if !self.filterable || self.query.trim().is_empty() {
            return items
                .iter()
                .map(|item| Shown {
                    item,
                    indices: Vec::new(),
                })
                .collect();
        }
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        let mut marks = vec![None; items.len()];
        for hit in crate::search::match_list(self.query.trim(), &names) {
            if let Some(slot) = marks.get_mut(hit.index) {
                *slot = Some(hit.indices);
            }
        }
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| marks[index].clone().map(|indices| Shown { item, indices }))
            .collect()
    }

    fn move_by(&mut self, step: usize, len: usize) -> MenuEvent {
        if len == 0 {
            return MenuEvent::Ignored;
        }
        self.active = (self.active + step) % len;
        MenuEvent::Moved
    }
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
