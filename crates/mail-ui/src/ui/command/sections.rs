//! ⌘K's panel, as sections: the rows `items` builds for one `search::run`, Recent, and the
//! places and Spaces the text names, regrouped under the titles the panel shows. Pure: a table
//! of answers in, the panel's rows out.
//!
//! With nothing typed the panel is the one way in: Recent (where the person was), then every
//! place, then the verbs with their keys. Typed, the text is the list's search and the list shows
//! the mail; the panel keeps the places, people and commands it names, so Return on one goes
//! there, and with none of them named Return leaves the panel for the list.

use ds::prelude::Icon;
use mail_client::search::{Results, match_list};

use super::super::menu::MenuItem;
use super::items::{self, Pick, interpret};
use mail_ui_model::copy::headers;

/// The section titles, in the order the panel draws them.
pub(in crate::ui) const RECENT: &str = headers::RECENT;
pub(in crate::ui) const PLACES: &str = headers::PLACES;
pub(in crate::ui) const PLACES_AND_PEOPLE: &str = headers::PLACES_AND_PEOPLE;
pub(in crate::ui) const COMMANDS: &str = headers::COMMANDS;
pub(in crate::ui) const TEMPLATES: &str = headers::TEMPLATES;

/// Rows each section keeps while something is typed: a panel is a glance, and the list under it
/// holds every match. With nothing typed every place and every command is listed.
const COMMANDS_CAP: usize = 8;
const PLACES_CAP: usize = 6;

/// What a pick in the panel does, read back from the row's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Choice {
    /// A recent conversation, a person or a command.
    Pick(Pick),
    /// The window's place at this index.
    Place(usize),
    /// The Space at this index.
    Space(usize),
    /// A draft put aside, from Recent: back exactly as it was left.
    Draft(mail_domain::DraftId),
}

/// One place a person can go: what it is called, its glyph, and whether it is listed with
/// nothing typed (one of mailo's own places, not History, and Scheduled only while a send waits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Destination {
    pub name: String,
    pub icon: Icon,
    pub at_rest: bool,
}

/// The panel's rows for `query`: `rows` (Recent's, and [`super::items::rows_of`]'s people and
/// commands) and `places` under the panel's titles, in the panel's order. With nothing typed:
/// Recent, Places, Commands. Typed: Places and People, then Commands, each capped. A row keeps
/// its tile: a person leads with their avatar, as quire's result rows draw one.
pub(in crate::ui) fn sections(
    rows: Vec<MenuItem>,
    places: Vec<MenuItem>,
    query: &str,
) -> Vec<MenuItem> {
    let empty = query.trim().is_empty();
    let titled = |row: MenuItem, title: &str| MenuItem {
        group: Some(title.to_owned()),
        ..row
    };
    let mut recent = Vec::new();
    let mut people = Vec::new();
    let mut commands = Vec::new();
    for row in rows {
        match row.group.as_deref() {
            Some(items::RECENT) if empty => recent.push(titled(row, RECENT)),
            Some(items::PEOPLE) if !empty => people.push(row),
            Some(items::ACTIONS) => commands.push(titled(row, COMMANDS)),
            _ => {}
        }
    }
    let found = if empty { PLACES } else { PLACES_AND_PEOPLE };
    let mut found: Vec<MenuItem> = places
        .into_iter()
        .chain(people)
        .map(|row| titled(row, found))
        .collect();
    if !empty {
        found.truncate(PLACES_CAP);
        commands.truncate(COMMANDS_CAP);
    }
    recent.into_iter().chain(found).chain(commands).collect()
}

/// The places ⌘K lists for `query`: with nothing typed, each place listed at rest, in the
/// sidebar's order; typed, the places and Spaces whose names it matches, best first, by the
/// matcher that marks names. The Space the window is in is no place to go, and is left out.
pub(in crate::ui) fn places_for(
    query: &str,
    places: &[Destination],
    spaces: &[String],
    current: usize,
) -> Vec<MenuItem> {
    let row = |key: String, icon, name: String, help, marks| MenuItem {
        help,
        group: Some(PLACES.to_owned()),
        marks,
        ..MenuItem::icon(key, icon, name)
    };
    if query.trim().is_empty() {
        return places
            .iter()
            .enumerate()
            .filter(|(_, place)| place.at_rest)
            .map(|(index, place)| {
                row(
                    format!("place:{index}"),
                    place.icon,
                    place.name.clone(),
                    None,
                    Vec::new(),
                )
            })
            .collect();
    }
    let names: Vec<&str> = places
        .iter()
        .map(|place| place.name.as_str())
        .chain(spaces.iter().map(String::as_str))
        .collect();
    match_list(query, &names)
        .into_iter()
        .filter_map(|hit| {
            let name = names.get(hit.index)?.to_string();
            match places.get(hit.index) {
                Some(place) => Some(row(
                    format!("place:{}", hit.index),
                    place.icon,
                    name,
                    None,
                    hit.indices,
                )),
                None => {
                    let space = hit.index - places.len();
                    (space != current).then(|| {
                        row(
                            format!("space:{space}"),
                            Icon::Grid,
                            name,
                            Some(headers::SPACE.to_owned()),
                            hit.indices,
                        )
                    })
                }
            }
        })
        .collect()
}

/// Whether typed `query` lights `row` before any key has moved the highlight: when the row's
/// name, or a word of it, starts with what is typed ("inb" names Inbox, "set" Settings…). A
/// looser match ("re" inside "Forward") leaves nothing lit, so Return leaves the panel for the
/// mail the list now shows rather than running a row nobody meant.
pub(in crate::ui) fn lights(row: &MenuItem, query: &str) -> bool {
    let typed = query.trim().to_lowercase();
    let name = row.name.to_lowercase();
    !typed.is_empty() && words_from(&name).any(|rest| rest.starts_with(&typed))
}

/// `name` from each of its words on: "add account" and "account".
fn words_from(name: &str) -> impl Iterator<Item = &str> {
    name.char_indices()
        .filter(move |(at, _)| *at == 0 || name[..*at].ends_with(char::is_whitespace))
        .map(move |(at, _)| &name[at..])
}

/// What the row keyed `key` does, against the answer that drew it.
pub(in crate::ui) fn choice_of(results: &Results, key: &str) -> Option<Choice> {
    let index = |rest: &str| rest.parse::<usize>().ok();
    match key.split_once(':') {
        Some(("place", rest)) => index(rest).map(Choice::Place),
        Some(("space", rest)) => index(rest).map(Choice::Space),
        Some(("recent", rest)) => {
            super::recent::thread_of(rest).map(|id| Choice::Pick(Pick::Open(id)))
        }
        Some(("draft", rest)) => super::recent::draft_of(rest).map(Choice::Draft),
        _ => interpret(results, key).map(Choice::Pick),
    }
}

/// The text Tab puts in the field for `row`: a command's or a place's name, and a person as the
/// search for their mail.
pub(in crate::ui) fn completion(row: &MenuItem) -> String {
    match row.key.split_once(':') {
        Some(("person", email)) => format!("from:{email}"),
        _ => row.name.clone(),
    }
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
