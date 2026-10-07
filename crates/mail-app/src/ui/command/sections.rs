//! The search bar's panel, as sections: the rows `items` builds for one `search::run`, the
//! places and Spaces the text names, regrouped under the titles the panel shows and capped to
//! a panel's length. Pure: a table of answers in, the panel's rows out.
//!
//! The ranker still decides the top hit, as it did for the ⌘K menu; it leads the panel, so
//! Return runs it. Under it, in a fixed order: Mail (Recent for an empty field), Commands, and
//! Places and People.

use ds::prelude::Icon;
use mail_core::search::{Results, match_list};

use super::super::menu::{MenuItem, Right, Tile};
use super::items::{Pick, interpret};

/// The section titles, in the order the panel draws them.
pub(in crate::ui) const TOP: &str = "Top Hit";
pub(in crate::ui) const MAIL: &str = "Mail";
pub(in crate::ui) const RECENT: &str = "Recent";
pub(in crate::ui) const COMMANDS: &str = "Commands";
pub(in crate::ui) const PLACES: &str = "Places and People";
pub(in crate::ui) const TEMPLATES: &str = "Templates";

/// Rows each section keeps: a panel is a glance, and the list under the bar holds every match.
const MAIL_CAP: usize = 5;
const COMMANDS_CAP: usize = 8;
const PLACES_CAP: usize = 6;

/// What a pick in the panel does, read back from the row's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Choice {
    /// A mail, a person or a command: what the ⌘K menu's row did.
    Pick(Pick),
    /// The window's place at this index.
    Place(usize),
    /// The Space at this index.
    Space(usize),
}

/// One place a person can go: what it is called, and its glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Destination {
    pub name: String,
    pub icon: Icon,
}

/// The panel's rows for `query`: `rows` (the ⌘K menu's, as [`super::items::rows_of`] builds
/// them) under the panel's titles, then `places`, each section capped, in the panel's order.
/// A row's tile is a glyph, since a menu row draws no avatar: a mail is an envelope with its
/// sender at the end, a person the people glyph.
pub(in crate::ui) fn sections(
    rows: Vec<MenuItem>,
    places: Vec<MenuItem>,
    query: &str,
) -> Vec<MenuItem> {
    let empty = query.trim().is_empty();
    let mut top = Vec::new();
    let mut mail = Vec::new();
    let mut commands = Vec::new();
    let mut people = Vec::new();
    for row in rows {
        let section = match row.group.as_deref() {
            // An empty field has no top hit: the newest thread is the first of the recent.
            Some("Top hit") if empty => RECENT,
            Some("Top hit") => TOP,
            Some("Actions") => COMMANDS,
            Some("People") => PLACES,
            _ if empty => RECENT,
            _ => MAIL,
        };
        let row = glyphed(MenuItem {
            group: Some(section.to_owned()),
            ..row
        });
        match section {
            TOP => top.push(row),
            COMMANDS => commands.push(row),
            PLACES => people.push(row),
            _ => mail.push(row),
        }
    }
    mail.truncate(MAIL_CAP);
    commands.truncate(COMMANDS_CAP);
    let mut found: Vec<MenuItem> = places
        .into_iter()
        .map(|row| MenuItem {
            group: Some(PLACES.to_owned()),
            ..row
        })
        .chain(people)
        .collect();
    found.truncate(PLACES_CAP);
    top.into_iter()
        .chain(mail)
        .chain(commands)
        .chain(found)
        .collect()
}

/// `row` with a glyph for its tile and its sender as the trailing word, as a menu row draws it.
fn glyphed(row: MenuItem) -> MenuItem {
    let tile = match &row.tile {
        Tile::Avatar { .. } if row.key.starts_with("mail:") => Tile::Icon(Icon::Mail),
        Tile::Avatar { .. } => Tile::Icon(Icon::Group),
        other => other.clone(),
    };
    // A mail's sender is the first run of its detail; a person's address is theirs.
    let help = match row.key.split_once(':') {
        Some(("mail", _)) => row.detail.first().map(|run| run.text.clone()),
        Some(("person", email)) if row.name != email => Some(email.to_owned()),
        _ => row.help.clone(),
    };
    MenuItem { tile, help, ..row }
}

/// The places and Spaces whose names `query` matches, best first, by the matcher the ⌘K menu
/// marks names with. A place a command already goes to (`offered`, "Go to Inbox") and the
/// Space the window is in are left out: each would be a second row doing what one does.
pub(in crate::ui) fn places_for(
    query: &str,
    places: &[Destination],
    spaces: &[String],
    current: usize,
    offered: &[String],
) -> Vec<MenuItem> {
    let names: Vec<&str> = places
        .iter()
        .map(|place| place.name.as_str())
        .chain(spaces.iter().map(String::as_str))
        .collect();
    match_list(query, &names)
        .into_iter()
        .filter_map(|hit| {
            let (key, icon, help) = match places.get(hit.index) {
                Some(place) if offered.contains(&format!("Go to {}", place.name)) => return None,
                Some(place) => (format!("place:{}", hit.index), place.icon, None),
                None => {
                    let space = hit.index - places.len();
                    if space == current {
                        return None;
                    }
                    (
                        format!("space:{space}"),
                        Icon::Grid,
                        Some("Space".to_owned()),
                    )
                }
            };
            Some(MenuItem {
                key,
                tile: Tile::Icon(icon),
                name: names.get(hit.index)?.to_string(),
                help,
                right: Right::None,
                group: Some(PLACES.to_owned()),
                marks: hit.indices,
                title: Vec::new(),
                detail: Vec::new(),
            })
        })
        .collect()
}

/// What the row keyed `key` does, against the answer that drew it.
pub(in crate::ui) fn choice_of(results: &Results, key: &str) -> Option<Choice> {
    let index = |rest: &str| rest.parse::<usize>().ok();
    match key.split_once(':') {
        Some(("place", rest)) => index(rest).map(Choice::Place),
        Some(("space", rest)) => index(rest).map(Choice::Space),
        _ => interpret(results, key).map(Choice::Pick),
    }
}

/// The text Tab puts in the field for `row`: a mail's subject, a command's or a place's name,
/// and a person as the search for their mail.
pub(in crate::ui) fn completion(row: &MenuItem) -> String {
    match row.key.split_once(':') {
        Some(("person", email)) => format!("from:{email}"),
        _ => row.name.clone(),
    }
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
