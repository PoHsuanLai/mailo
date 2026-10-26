//! ⌘K's command and people rows, built from one `search::run`. The mail it finds is the list's:
//! typing in ⌘K is the list's search, so the panel draws no mail of its own.
//!
//! The ranker's sender affinity comes from the one grouped sender history the hover cards read
//! (`ui::history`): how many conversations, and whether you have written to them.
//! [`mail_core::search::Affinity::default`] is an empty map, and an empty map has no people, so
//! "dana" would never be a person. Who the People rows are is the contact book's answer
//! (`people.rs`), the one the composer's To field gets.

use ds::prelude::*;
use std::collections::HashMap;

use super::super::menu::{MenuItem, Right, Run, Tile, Tone};
use crate::ui::actions;
use crate::ui::bin::Bin;
use crate::ui::copy::menus;
use chordkit::{Keymap, StandardAction};
use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::search::{self, ActionHit, Command, PersonHit, Results, Top};
use mail_domain::ThreadId;

/// A command the panel offers, by what it does. A row is dispatched on this, never on its
/// words: its label is looked up from [`ENTRIES`], so renaming one changes no behaviour.
///
/// Only verbs. A place is found by its own name ([`super::sections::places_for`]), and a page of
/// Settings, the theme or the state of the connections are Settings' own, one row away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum CommandId {
    Compose,
    FromTemplate,
    ForwardAttached,
    Print,
    Sync,
    Empty(Bin),
    TidyUp,
    Sidebar,
    AddAccount,
    Import,
    Export,
    NewView,
    Settings,
    /// Contacts in or out as vCard: the book has no page of its own.
    Contacts(Cards),
}

/// Which way a Contacts command moves the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Cards {
    /// Settings › Contacts, the book itself.
    Open,
    Import,
    Export,
}

/// One command's row: what it does, the key its row carries, its words, its glyph, and the
/// standard action whose chord is drawn beside it.
pub(in crate::ui) struct Entry {
    pub id: CommandId,
    /// The row's key after `action:`. Stable, and never shown.
    pub key: &'static str,
    pub label: &'static str,
    pub icon: Icon,
    pub standard: Option<StandardAction>,
}

const fn entry(
    id: CommandId,
    key: &'static str,
    label: &'static str,
    icon: Icon,
    standard: Option<StandardAction>,
) -> Entry {
    Entry {
        id,
        key,
        label,
        icon,
        standard,
    }
}

/// The actions the window can run today, in the order the panel offers them.
pub(in crate::ui) const ENTRIES: &[Entry] = &[
    entry(
        CommandId::Compose,
        "compose",
        menus::NEW_MESSAGE,
        Icon::Pen,
        Some(StandardAction::New),
    ),
    entry(
        CommandId::FromTemplate,
        "from-template",
        menus::NEW_FROM_TEMPLATE,
        Icon::FilePen,
        None,
    ),
    entry(
        CommandId::ForwardAttached,
        "forward-attached",
        menus::FORWARD_AS_ATTACHMENT,
        Icon::Forward,
        None,
    ),
    entry(
        CommandId::Print,
        "print",
        menus::PRINT_CONVERSATION,
        Icon::Printer,
        Some(StandardAction::Print),
    ),
    entry(
        CommandId::Sync,
        "sync",
        menus::SYNC_NOW,
        Icon::Refresh,
        None,
    ),
    entry(
        CommandId::Empty(Bin::Trash),
        "empty-trash",
        menus::EMPTY_TRASH,
        Icon::Command,
        None,
    ),
    entry(
        CommandId::Empty(Bin::Spam),
        "empty-spam",
        menus::EMPTY_SPAM,
        Icon::Command,
        None,
    ),
    entry(
        CommandId::TidyUp,
        "tidy-up",
        menus::TIDY_UP,
        Icon::Archive,
        None,
    ),
    entry(
        CommandId::Sidebar,
        "sidebar",
        menus::HIDE_SIDEBAR,
        Icon::PanelLeft,
        Some(StandardAction::ToggleSidebar),
    ),
    entry(
        CommandId::AddAccount,
        "add-account",
        menus::ADD_ACCOUNT,
        Icon::Plus,
        None,
    ),
    entry(
        CommandId::Import,
        "import",
        menus::IMPORT_MAIL,
        Icon::Plus,
        None,
    ),
    entry(
        CommandId::Export,
        "export",
        menus::EXPORT_MAIL,
        Icon::Forward,
        None,
    ),
    // The contact book, and the book in and out as vCard.
    entry(
        CommandId::Contacts(Cards::Open),
        "contacts",
        menus::OPEN_CONTACTS,
        Icon::Group,
        None,
    ),
    entry(
        CommandId::Contacts(Cards::Import),
        "import-contacts",
        menus::IMPORT_CONTACTS,
        Icon::Group,
        None,
    ),
    entry(
        CommandId::Contacts(Cards::Export),
        "export-contacts",
        menus::EXPORT_CONTACTS,
        Icon::Group,
        None,
    ),
    entry(
        CommandId::NewView,
        "new-view",
        menus::NEW_VIEW,
        Icon::Search,
        None,
    ),
    entry(
        CommandId::Settings,
        "settings",
        menus::SETTINGS,
        Icon::Settings,
        None,
    ),
];

/// The commands as the matcher takes them: their words, in the panel's order.
pub(in crate::ui) fn commands() -> Vec<Command> {
    ENTRIES
        .iter()
        .map(|entry| Command {
            label: entry.label.to_owned(),
        })
        .collect()
}

/// The entry for `id`. Every id has one; [`ENTRIES`] is checked against them in the tests.
pub(in crate::ui) fn entry_of(id: CommandId) -> &'static Entry {
    ENTRIES
        .iter()
        .find(|entry| entry.id == id)
        .expect("every command has an entry")
}

/// The entry the matcher answered with: its hit carries the words it matched, which are one
/// entry's (the labels are distinct, as the tests hold).
fn entry_hit(hit: &ActionHit) -> Option<&'static Entry> {
    ENTRIES
        .iter()
        .find(|entry| entry.label == hit.command.label)
}

/// The key a command's row carries.
pub(in crate::ui) fn action_key(entry: &Entry) -> String {
    format!("action:{}", entry.key)
}

/// What Enter does with one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    Open(ThreadId),
    From(String),
    Action(CommandId),
}

/// The key the row was given, read back against the results that produced it.
pub(in crate::ui) fn interpret(results: &Results, key: &str) -> Option<Pick> {
    let person = |email: &str| format!("person:{}", email.to_ascii_lowercase());
    let top_person = match &results.top {
        Some(Top::Person(hit)) => Some(hit),
        _ => None,
    };
    for hit in top_person.into_iter().chain(&results.people) {
        if key == person(&hit.email) {
            return Some(Pick::From(hit.email.to_ascii_lowercase()));
        }
    }
    let top_action = match &results.top {
        Some(Top::Action(hit)) => Some(hit),
        _ => None,
    };
    top_action
        .into_iter()
        .chain(&results.actions)
        .filter_map(entry_hit)
        .find(|entry| key == action_key(entry))
        .map(|entry| Pick::Action(entry.id))
}

/// The groups [`rows_of`] and Recent file their rows under, which
/// [`super::sections::sections`] regroups by. Not shown: the panel draws its own titles.
pub(in crate::ui) const RECENT: &str = "recent";
pub(in crate::ui) const PEOPLE: &str = "people";
pub(in crate::ui) const ACTIONS: &str = "actions";

/// The rows the panel paints for one `search::run`: its people and its commands, the one the
/// ranker put on top first among its kind. Its mail is not here: what is typed is the list's
/// search, and the list shows that mail.
pub(in crate::ui) fn rows_of(
    results: &Results,
    names: &HashMap<String, String>,
    query: &str,
    keymap: &Keymap,
) -> Vec<MenuItem> {
    let (top_person, top_action) = match &results.top {
        Some(Top::Person(hit)) => (Some(hit), None),
        Some(Top::Action(hit)) => (None, Some(hit)),
        Some(Top::Mail(_)) | None => (None, None),
    };
    let people = top_person
        .into_iter()
        .chain(&results.people)
        .map(|hit| person_item(hit, names, query, PEOPLE));
    let actions = top_action
        .into_iter()
        .chain(&results.actions)
        .map(|hit| action_item(hit, ACTIONS, keymap));
    people.chain(actions).collect()
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
    item.name = menus::SHOW_SIDEBAR.to_owned();
    item.marks = query_marks(query, &item.name);
    item
}

/// The key of the sidebar's row, which is the same whichever way it reads.
pub(in crate::ui) const SIDEBAR_KEY: &str = "action:sidebar";

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

/// A command row, with the chord of the standard action it is drawn from `keymap`: its first
/// chord, in the platform's own order and words (`Ctrl+N` on Windows, ⌘N on a Mac).
fn action_item(hit: &ActionHit, group: &str, keymap: &Keymap) -> MenuItem {
    let entry = entry_hit(hit);
    let shortcut = entry
        .and_then(|entry| entry.standard)
        .and_then(|action| actions::hint_standard(keymap, action));
    MenuItem {
        right: shortcut.map(Right::Shortcut).unwrap_or(Right::None),
        group: Some(group.to_owned()),
        marks: hit.indices.clone(),
        ..MenuItem::icon(
            entry.map_or_else(String::new, action_key),
            entry.map_or(Icon::Command, |entry| entry.icon),
            hit.command.label.clone(),
        )
    }
}

fn initial(name: &str) -> char {
    name.chars().next().unwrap_or('·').to_ascii_uppercase()
}

/// The search's syntax, as ⌘K suggests it: the operators a person can type, most used first.
pub(in crate::ui) const SYNTAX: &[&str] = &[
    "from:",
    "to:",
    "subject:",
    "has:attachment",
    "is:unread",
    "is:starred",
    "label:",
    "before:",
    "after:",
];

/// The chips under the field for `query`: the whole syntax while nothing is typed, the operators
/// the last word is the start of while it is one ("fr" is offered "from:", "is:" both of its
/// kinds), and otherwise the operators the search holds, so the person sees what narrows it.
pub(in crate::ui) fn chips(query: &str) -> Vec<String> {
    if query.trim().is_empty() {
        return SYNTAX.iter().map(|chip| (*chip).to_owned()).collect();
    }
    let started = starting(query);
    if !started.is_empty() {
        return started.into_iter().map(str::to_owned).collect();
    }
    search::parse(query, &Utc, &|_| Vec::new()).operator_tokens
}

/// The operators `query`'s last word is the start of, and not yet one of: none when the query
/// ends in a space, the word is whole, or it starts nothing.
fn starting(query: &str) -> Vec<&'static str> {
    if query.ends_with(char::is_whitespace) {
        return Vec::new();
    }
    let last = query
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if last.is_empty() {
        return Vec::new();
    }
    SYNTAX
        .iter()
        .copied()
        .filter(|chip| chip.starts_with(&last) && *chip != last)
        .collect()
}

/// `query` with its last word completed to the first operator it starts, for Tab; `None` when
/// it starts none.
pub(in crate::ui) fn complete_syntax(query: &str) -> Option<String> {
    let chip = starting(query).into_iter().next()?;
    let kept = query.trim_end_matches(|c: char| !c.is_whitespace());
    Some(format!("{kept}{chip}"))
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
