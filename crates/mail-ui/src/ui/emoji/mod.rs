//! The emoji the composer offers, and how a typed name finds one.
//!
//! The table is Unicode's: every fully-qualified emoji up to Emoji 15.0 in CLDR order, with its
//! CLDR short name and English keywords, less skin-tone sequences (`table.txt`, written by
//! `scripts/emoji-table.py`, with its licence notice). It is read once, borrowing from the
//! compiled-in text.
//!
//! [`trigger`] says whether the text before the caret ends in a `:name` being typed, and
//! [`search`] what that name finds. [`recent`] is the user's last picks, kept per user.

pub mod recent;

#[cfg(test)]
mod tests;

use std::sync::LazyLock;

use unicode_segmentation::UnicodeSegmentation;

/// The table, as written by `scripts/emoji-table.py`.
const TABLE: &str = include_str!("table.txt");

/// The fewest characters after a `:` before it offers anything: a `:` alone, or one letter after
/// it, is still ordinary writing ("Note :x").
pub const SHORTEST_QUERY: usize = 2;

/// The longest name a `:` follows before it stops offering.
pub const LONGEST_QUERY: usize = 32;

/// The groups of the table, in its order: the picker's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Smileys,
    People,
    Animals,
    Food,
    Travel,
    Activities,
    Objects,
    Symbols,
    Flags,
}

impl Group {
    /// Every group, in the table's order.
    pub const ALL: [Group; 9] = [
        Group::Smileys,
        Group::People,
        Group::Animals,
        Group::Food,
        Group::Travel,
        Group::Activities,
        Group::Objects,
        Group::Symbols,
        Group::Flags,
    ];

    /// The group's name as Unicode gives it.
    pub fn title(self) -> &'static str {
        match self {
            Group::Smileys => "Smileys & Emotion",
            Group::People => "People & Body",
            Group::Animals => "Animals & Nature",
            Group::Food => "Food & Drink",
            Group::Travel => "Travel & Places",
            Group::Activities => "Activities",
            Group::Objects => "Objects",
            Group::Symbols => "Symbols",
            Group::Flags => "Flags",
        }
    }

    /// The emoji that stands for the group on its tab.
    pub fn face(self) -> &'static str {
        match self {
            Group::Smileys => "😀",
            Group::People => "👋",
            Group::Animals => "🐻",
            Group::Food => "🍔",
            Group::Travel => "🚗",
            Group::Activities => "⚽",
            Group::Objects => "💡",
            Group::Symbols => "🔣",
            Group::Flags => "🏁",
        }
    }

    fn of_title(title: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|group| group.title() == title)
    }
}

/// One emoji of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emoji {
    /// The emoji itself, as it is inserted.
    pub glyph: &'static str,
    /// Its CLDR short name ("smiling face with smiling eyes").
    pub name: &'static str,
    /// CLDR's English keywords that are not already words of the name, space-separated.
    pub keywords: &'static str,
    pub group: Group,
}

static ALL: LazyLock<Vec<Emoji>> = LazyLock::new(|| parse(TABLE));

/// Read the table: `= Group` lines, then `glyph<TAB>name[<TAB>keywords]` lines. A line under a
/// group the table does not know, or without a name, is skipped; a test holds that none is.
fn parse(table: &'static str) -> Vec<Emoji> {
    let mut group = None;
    let mut out = Vec::new();
    for line in table.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(title) = line.strip_prefix("= ") {
            group = Group::of_title(title);
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(group), Some(glyph), Some(name)) = (group, fields.next(), fields.next()) else {
            continue;
        };
        out.push(Emoji {
            glyph,
            name,
            keywords: fields.next().unwrap_or(""),
            group,
        });
    }
    out
}

/// Every emoji, in the table's order.
pub fn all() -> &'static [Emoji] {
    &ALL
}

/// The emoji of `group`, in order.
pub fn in_group(group: Group) -> impl Iterator<Item = &'static Emoji> {
    ALL.iter().filter(move |emoji| emoji.group == group)
}

/// The table's entry for `glyph`, if it has one.
pub fn find(glyph: &str) -> Option<&'static Emoji> {
    ALL.iter().find(|emoji| emoji.glyph == glyph)
}

/// A `:name` being typed: where its `:` is, in graphemes of the text given, and the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger<'a> {
    pub at: usize,
    pub query: &'a str,
}

/// Whether `before`, the paragraph's text up to the caret, ends in a `:name` being typed.
///
/// The `:` starts a word (the line's start or after a space), so "Note:", "10:30" and
/// "https://" never offer anything. The name is [`SHORTEST_QUERY`] to [`LONGEST_QUERY`]
/// letters, digits, `_`, `-` or `+`, so a space or any other mark ends it, and a smiley typed as
/// `:)` or `:-(` offers nothing.
pub fn trigger(before: &str) -> Option<Trigger<'_>> {
    let graphemes: Vec<(usize, &str)> = before.grapheme_indices(true).collect();
    let colon = graphemes
        .iter()
        .rposition(|(_, grapheme)| *grapheme == ":")?;
    let query = &before[graphemes[colon].0 + 1..];
    let length = query.chars().count();
    let named = query
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+'));
    let starts_word = colon
        .checked_sub(1)
        .is_none_or(|previous| graphemes[previous].1.chars().all(char::is_whitespace));
    ((SHORTEST_QUERY..=LONGEST_QUERY).contains(&length) && named && starts_word)
        .then_some(Trigger { at: colon, query })
}

/// The words of a name or of keywords, lower case: split at anything not a letter, a digit or
/// `+` (so `heart-eyes` is two words and `+1` is one).
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !(ch.is_alphanumeric() || ch == '+'))
        .filter(|word| !word.is_empty())
}

/// How well an emoji answers a query, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// The name is the query.
    Name,
    /// Each word of the query is a whole word of the name.
    NameWhole,
    /// Each word of the query is a whole word of the name or a keyword: `lol` is laughing
    /// before it is a lollipop.
    Words,
    /// The name starts with the query.
    NameStart,
    /// Each word of the query starts a word of the name.
    NameWords,
    /// Each word of the query starts a word of the name or of a keyword.
    Keywords,
}

fn fit(emoji: &Emoji, query: &str, parts: &[&str]) -> Option<Fit> {
    let name = emoji.name.to_lowercase();
    if name == query {
        return Some(Fit::Name);
    }
    let starts_one = |words: &[&str], part: &str| words.iter().any(|word| word.starts_with(part));
    let named: Vec<&str> = words(&name).collect();
    let mut every: Vec<&str> = named.clone();
    every.extend(words(emoji.keywords));
    if parts.iter().all(|part| named.contains(part)) {
        return Some(Fit::NameWhole);
    }
    if parts.iter().all(|part| every.contains(part)) {
        return Some(Fit::Words);
    }
    if name.starts_with(query) {
        return Some(Fit::NameStart);
    }
    if parts.iter().all(|part| starts_one(&named, part)) {
        return Some(Fit::NameWords);
    }
    parts
        .iter()
        .all(|part| starts_one(&every, part))
        .then_some(Fit::Keywords)
}

/// The emoji `query` names, best first, then in the table's order.
///
/// `_` reads as a space, as in `:thumbs_up`, and case does not count. Matching is by word
/// start, never inside a word: `ear` finds the ear and not a bear.
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let query = query.replace('_', " ").to_lowercase();
    let parts: Vec<&str> = words(&query).collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let query = parts.join(" ");
    let mut found: Vec<(Fit, usize, &'static Emoji)> = ALL
        .iter()
        .enumerate()
        .filter_map(|(index, emoji)| fit(emoji, &query, &parts).map(|fit| (fit, index, emoji)))
        .collect();
    found.sort_by_key(|(fit, index, _)| (*fit, *index));
    found.into_iter().map(|(_, _, emoji)| emoji).collect()
}
