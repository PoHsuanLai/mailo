//! The search bar's rows, built from one `search::run`, as the ⌘K menu built them.
//!
//! The ranker's sender affinity comes from the one grouped sender history the hover cards read
//! (`ui::history`): how many conversations, and whether you have written to them.
//! [`mail_core::search::Affinity::default`] is an empty map, and an empty map has no people, so
//! "dana" would never be a person. Who the People rows are is the contact book's answer
//! (`people.rs`), the one the composer's To field gets.

use ds::prelude::*;
use std::collections::HashMap;

use super::super::menu::{MenuItem, Right, Run, Tile, Tone};
use chrono::{DateTime, Utc};
use mail_core::search::{self, ActionHit, Command, MailHit, PersonHit, Results, Top};
use mail_domain::ThreadId;
use mail_store::SqliteStore;

/// The actions the window can run today.
pub(in crate::ui) fn commands() -> Vec<Command> {
    [
        "Compose",
        "New from template",
        "Forward as attachment",
        "Print conversation",
        "Sync now",
        "Connection Doctor",
        "Go to Inbox",
        "Go to Starred",
        "Go to Snoozed",
        "Go to Archive",
        "Go to Trash",
        "Empty Trash…",
        "Empty Spam…",
        "Hide sidebar",
        "Add account…",
        "Import mail…",
        "Export mail…",
        "New view…",
        "Settings…",
        // Each page of Settings, named so that "set" finds them all.
        "General Settings",
        "Accounts Settings",
        "Contacts Settings",
        "Rules Settings",
        "Keys and Certificates Settings",
        "Keyboard Shortcuts Settings",
        "Theme light",
        "Theme dark",
        "Theme system",
    ]
    .into_iter()
    .map(|label| Command {
        label: label.to_owned(),
    })
    .collect()
}

/// What Enter does with one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    Open(ThreadId),
    From(String),
    Action(String),
}

/// The key the row was given, read back against the results that produced it.
pub(in crate::ui) fn interpret(results: &Results, key: &str) -> Option<Pick> {
    let mail_id = |hit: &MailHit| format!("mail:{}", hit.summary.id);
    if let Some(top) = &results.top {
        match top {
            Top::Mail(hit) if key == mail_id(hit) => return Some(Pick::Open(hit.summary.id)),
            Top::Person(hit) if key == format!("person:{}", hit.email.to_ascii_lowercase()) => {
                return Some(Pick::From(hit.email.to_ascii_lowercase()));
            }
            Top::Action(hit) if key == format!("action:{}", hit.command.label) => {
                return Some(Pick::Action(hit.command.label.clone()));
            }
            _ => {}
        }
    }
    for hit in &results.mail {
        if key == mail_id(hit) {
            return Some(Pick::Open(hit.summary.id));
        }
    }
    for hit in &results.people {
        if key == format!("person:{}", hit.email.to_ascii_lowercase()) {
            return Some(Pick::From(hit.email.to_ascii_lowercase()));
        }
    }
    for hit in &results.actions {
        if key == format!("action:{}", hit.command.label) {
            return Some(Pick::Action(hit.command.label.clone()));
        }
    }
    None
}

/// The rows the menu paints for one `search::run`.
pub(in crate::ui) fn rows_of(
    results: &Results,
    names: &HashMap<String, String>,
    query: &str,
) -> Vec<MenuItem> {
    let mut out = Vec::new();
    if let Some(top) = &results.top {
        out.push(top_item(top, names, query));
    }
    let mail_group = if query.trim().is_empty() {
        "Recent"
    } else {
        "Mail"
    };
    for hit in &results.mail {
        out.push(mail_item(hit, mail_group, query));
    }
    for hit in &results.people {
        out.push(person_item(hit, names, query, "People"));
    }
    for hit in &results.actions {
        out.push(action_item(hit, "Actions"));
    }
    out
}

fn top_item(top: &Top, names: &HashMap<String, String>, query: &str) -> MenuItem {
    match top {
        Top::Mail(hit) => mail_item(hit, "Top hit", query),
        Top::Person(hit) => person_item(hit, names, query, "Top hit"),
        Top::Action(hit) => action_item(hit, "Top hit"),
    }
}

fn mail_item(hit: &MailHit, group: &str, query: &str) -> MenuItem {
    let who = hit
        .summary
        .from
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| hit.summary.from.email.clone());
    let letter = initial(&who);
    MenuItem {
        key: format!("mail:{}", hit.summary.id),
        tile: Tile::Avatar {
            letter,
            color: avatar_color(&hit.summary.from.email),
        },
        name: hit.summary.subject.clone(),
        help: None,
        right: Right::None,
        group: Some(group.to_owned()),
        marks: char_marks(&hit.summary.subject, &hit.indices, &hit.marks),
        title: Vec::new(),
        detail: mail_detail(who, &hit.preview, query),
    }
}

/// A mail row's second line: its sender, then its snippet when it has one.
fn mail_detail(who: String, preview: &str, query: &str) -> Vec<Run> {
    let mut runs = vec![Run {
        marks: query_marks(query, &who),
        text: who,
        tone: Tone::Strong,
    }];
    if !preview.trim().is_empty() {
        runs.push(Run {
            text: format!(" · {preview}"),
            marks: Vec::new(),
            tone: Tone::Plain,
        });
    }
    runs
}

fn char_marks(text: &str, indices: &[u32], ranges: &[std::ops::Range<usize>]) -> Vec<u32> {
    if !indices.is_empty() {
        return indices.to_vec();
    }
    let mut out = Vec::new();
    for (index, (byte, _)) in text.char_indices().enumerate() {
        if ranges
            .iter()
            .any(|range| byte >= range.start && byte < range.end)
        {
            out.push(index as u32);
        }
    }
    out
}

fn person_item(
    hit: &PersonHit,
    names: &HashMap<String, String>,
    query: &str,
    group: &str,
) -> MenuItem {
    let email = hit.email.to_ascii_lowercase();
    let name = names
        .get(&email)
        .cloned()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| hit.email.clone());
    let threads = if hit.threads == 0 {
        "no mail from them".to_owned()
    } else if hit.threads == 1 {
        "1 thread".to_owned()
    } else {
        format!("{} threads", hit.threads)
    };
    let name_marks = query_marks(query, &name);
    let addr_marks = query_marks(query, &email);
    MenuItem {
        key: format!("person:{email}"),
        tile: Tile::Avatar {
            letter: initial(&name),
            color: avatar_color(&email),
        },
        name: name.clone(),
        help: None,
        right: Right::None,
        group: Some(group.to_owned()),
        marks: name_marks.clone(),
        title: vec![
            Run {
                text: name,
                marks: name_marks,
                tone: Tone::Plain,
            },
            Run {
                text: " ".to_owned(),
                marks: Vec::new(),
                tone: Tone::Plain,
            },
            Run {
                text: email.clone(),
                marks: addr_marks,
                tone: Tone::Faint,
            },
        ],
        detail: vec![Run {
            text: format!("{email} · {threads}"),
            marks: Vec::new(),
            tone: Tone::Plain,
        }],
    }
}

/// The sidebar row, worded for the sidebar as it is: "Hide sidebar" while it is pinned, "Show
/// sidebar" while it is hidden. The key (and so what a pick does) stays the same; the marks are
/// the query's against the new wording, since the old ones indexed the old one.
pub(in crate::ui) fn restate_sidebar(mut item: MenuItem, sidebar: Shown, query: &str) -> MenuItem {
    if item.key != SIDEBAR_KEY || sidebar == Shown::Visible {
        return item;
    }
    item.name = "Show sidebar".to_owned();
    item.marks = query_marks(query, &item.name);
    item
}

/// The key of the sidebar's row, which is the same whichever way it reads.
pub(in crate::ui) const SIDEBAR_KEY: &str = "action:Hide sidebar";

fn query_marks(query: &str, text: &str) -> Vec<u32> {
    search::match_list(query, &[text])
        .into_iter()
        .next()
        .map(|hit| hit.indices)
        .unwrap_or_default()
}

/// A person's avatar fill, stable for an address: quire's person hash (design/03 section 13),
/// the colour its `Avatar` gives the same address, as the text a tile's style takes.
pub(in crate::ui) fn avatar_color(email: &str) -> String {
    ds::components::content::avatar::person_hue(email)
        .hex()
        .css()
}

fn action_item(hit: &ActionHit, group: &str) -> MenuItem {
    let shortcut = match hit.command.label.as_str() {
        "Compose" => Some("\u{2318}N".to_owned()),
        "Hide sidebar" => Some("\u{2303}\u{2318}S".to_owned()),
        "Print conversation" => Some("\u{2318}P".to_owned()),
        _ => None,
    };
    MenuItem {
        key: format!("action:{}", hit.command.label),
        tile: Tile::Icon(action_icon(&hit.command.label)),
        name: hit.command.label.clone(),
        help: None,
        right: shortcut.map(Right::Shortcut).unwrap_or(Right::None),
        group: Some(group.to_owned()),
        marks: hit.indices.clone(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

fn action_icon(label: &str) -> Icon {
    match label {
        "Compose" => Icon::Pen,
        "New from template" => Icon::FilePen,
        "Print conversation" => Icon::Printer,
        "Forward as attachment" => Icon::Forward,
        "Sync now" => Icon::Refresh,
        "Connection Doctor" => Icon::TriangleAlert,
        "Hide sidebar" => Icon::PanelLeft,
        "Theme light" | "Theme dark" | "Theme system" => Icon::Settings,
        "Go to Inbox" => Icon::Inbox,
        "Go to Starred" => Icon::Star,
        "Go to Snoozed" => Icon::Clock,
        "Go to Archive" => Icon::Archive,
        "Go to Trash" => Icon::Trash,
        "Contacts Settings" => Icon::Group,
        "Accounts Settings" => Icon::Mail,
        "Add account…" => Icon::Plus,
        "Import mail…" => Icon::Plus,
        "Export mail…" => Icon::Forward,
        "Rules Settings" => Icon::FolderInput,
        "New view…" => Icon::Search,
        "Keys and Certificates Settings" => Icon::Key,
        "Keyboard Shortcuts Settings" => Icon::Keyboard,
        "Settings…" | "General Settings" => Icon::Settings,
        _ => Icon::Command,
    }
}

fn initial(name: &str) -> char {
    name.chars().next().unwrap_or('·').to_ascii_uppercase()
}

/// Operator tokens in `query`, as the chips under the field.
pub(in crate::ui) fn tokens(query: &str) -> Vec<String> {
    search::parse(query, &Utc, &|_| Vec::new()).operator_tokens
}

/// Run the menu's search. Its People are the contact book's, as [`super::people::from_book`]
/// puts them.
pub(in crate::ui) fn search_now(
    store: &SqliteStore,
    query: &str,
    now: DateTime<Utc>,
) -> (Results, HashMap<String, String>) {
    let history = super::super::history::history(store);
    let (affinity, mut names) = (history.affinity(), history.names());
    let mut results = search::run(query, store, &affinity, &commands(), now);
    super::people::from_book(&mut results, &mut names, store, query, &history);
    (results, names)
}

#[cfg(test)]
mod detail_tests {
    use super::mail_detail;

    #[test]
    fn a_mail_with_no_snippet_is_its_sender_alone() {
        let cases: &[(&str, &str)] = &[
            ("see you at noon", "Dana · see you at noon"),
            ("", "Dana"),
            ("  ", "Dana"),
        ];
        for (preview, want) in cases {
            let line: String = mail_detail("Dana".to_owned(), preview, "")
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(line, *want, "{preview:?}");
        }
    }
}
