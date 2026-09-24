//! The rows of the Ctrl T menu, built from one `search::run`.
//!
//! The ranker's sender affinity comes from the one grouped sender history the hover cards read
//! (`ui::history`): how many conversations, and whether you have written to them.
//! [`crate::search::Affinity::default`] is an empty map, and an empty map has no people, so
//! "dana" would never be a person. Who the People rows are is the contact book's answer
//! (`people.rs`), the one the composer's To field gets.

use std::collections::HashMap;

use super::super::menu::{MenuItem, Right, Run, Tile, Tone};
use crate::search::{self, ActionHit, Command, MailHit, PersonHit, Results, Top};
use chrono::{DateTime, Utc};
use ds::Icon;
use mail_domain::ThreadId;
use mail_store::SqliteStore;

/// The actions the window can run today.
pub(in crate::ui) fn commands() -> Vec<Command> {
    [
        "Compose",
        "New from template",
        "Print conversation",
        "Sync now",
        "Go to Inbox",
        "Go to Starred",
        "Go to Snoozed",
        "Go to Archive",
        "Go to Trash",
        "Hide sidebar",
        "Contacts",
        "Add account…",
        "Import mail…",
        "Export mail…",
        "Rules…",
        "Keys and certificates…",
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
        detail: vec![
            Run {
                marks: query_marks(query, &who),
                text: who,
                tone: Tone::Strong,
            },
            Run {
                text: format!(" · {}", hit.preview),
                marks: Vec::new(),
                tone: Tone::Plain,
            },
        ],
    }
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

fn query_marks(query: &str, text: &str) -> Vec<u32> {
    search::match_list(query, &[text])
        .into_iter()
        .next()
        .map(|hit| hit.indices)
        .unwrap_or_default()
}

/// The eight avatar fills `tokens.css` declared as `--av-0` to `--av-7`.
///
/// quire has no token for them: its person colour is the `Avatar` component's own hash
/// (`ds::PersonHue`, whose colour is not public), and these tiles move to that component with
/// the menus. Until then the fills are the same eight, written as values, so the window looks
/// as it did; a gap reported to quire, not a token mailo declares.
const AVATAR_FILLS: [&str; 8] = [
    "#5B4B8A", "#1F6B4A", "#8A4B2F", "#2F5D8A", "#8A3D55", "#3D6B4F", "#6B5420", "#3E4A78",
];

/// One of eight avatar fills, stable for an address.
pub(in crate::ui) fn avatar_color(email: &str) -> String {
    let mut hash = 2166136261u32;
    for byte in email.to_ascii_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16777619);
    }
    let index = usize::try_from(hash).unwrap_or_default() % AVATAR_FILLS.len();
    AVATAR_FILLS[index].to_owned()
}

fn action_item(hit: &ActionHit, group: &str) -> MenuItem {
    let shortcut = match hit.command.label.as_str() {
        "Compose" => Some("C".to_owned()),
        "Hide sidebar" => Some("Ctrl S".to_owned()),
        "Print conversation" => Some("Ctrl P".to_owned()),
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
        "Print conversation" => crate::ui::PRINTER,
        "Sync now" => Icon::Refresh,
        "Hide sidebar" => Icon::PanelLeft,
        "Theme light" | "Theme dark" | "Theme system" => Icon::Settings,
        "Go to Inbox" => Icon::Inbox,
        "Go to Starred" => Icon::Star,
        "Go to Snoozed" => Icon::Clock,
        "Go to Archive" => Icon::Archive,
        "Go to Trash" => Icon::Trash,
        "Contacts" => Icon::Group,
        "Add account…" => Icon::Plus,
        "Import mail…" => Icon::Plus,
        "Export mail…" => Icon::Forward,
        "Rules…" => crate::ui::FOLDER_INPUT,
        "Keys and certificates…" => Icon::Key,
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
