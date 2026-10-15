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
use crate::ui::actions;
use crate::ui::bin::Bin;
use crate::ui::copy;
use crate::ui::view::{SettingsPage, Source};
use chordkit::{Keymap, StandardAction};
use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::place::{pending_snooze, place_filter};
use mail_core::search::{self, ActionHit, Command, MailHit, PersonHit, Results, Top};
use mail_core::view::kept_filter;
use mail_domain::{MailboxRole, ThreadId};

/// A command the panel offers, by what it does. A row is dispatched on this, never on its
/// words: its label is looked up from [`ENTRIES`], so renaming one changes no behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum CommandId {
    Compose,
    FromTemplate,
    ForwardAttached,
    Print,
    Sync,
    Connections,
    Go(Goes),
    Empty(Bin),
    TidyUp,
    Sidebar,
    AddAccount,
    Import,
    Export,
    NewView,
    Settings,
    SettingsAt(SettingsPage),
    Theme(Look),
    /// Contacts in or out as vCard: the book has no page of its own.
    Contacts(Cards),
}

/// Which way a Contacts command moves the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Cards {
    Import,
    Export,
}

/// A place a Go command opens: one of the sidebar's own, found by what it lists rather than by
/// what it is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Goes {
    Inbox,
    Kept,
    Snoozed,
    Done,
    Trash,
}

impl Goes {
    /// What the place lists, which is how it is found among the window's places.
    pub(in crate::ui) fn source(self) -> Source {
        match self {
            Goes::Inbox => Source::Mail(place_filter(MailboxRole::Inbox)),
            Goes::Kept => Source::Mail(kept_filter()),
            Goes::Snoozed => Source::Mail(pending_snooze()),
            Goes::Done => Source::Mail(place_filter(MailboxRole::Archive)),
            Goes::Trash => Source::Mail(place_filter(MailboxRole::Trash)),
        }
    }
}

/// An appearance a Theme command sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Look {
    Light,
    Dark,
    System,
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
        "Compose",
        Icon::Pen,
        Some(StandardAction::New),
    ),
    entry(
        CommandId::FromTemplate,
        "from-template",
        "New from template",
        Icon::FilePen,
        None,
    ),
    entry(
        CommandId::ForwardAttached,
        "forward-attached",
        "Forward as attachment",
        Icon::Forward,
        None,
    ),
    entry(
        CommandId::Print,
        "print",
        "Print conversation",
        Icon::Printer,
        Some(StandardAction::Print),
    ),
    entry(CommandId::Sync, "sync", "Sync now", Icon::Refresh, None),
    entry(
        CommandId::Connections,
        "connections",
        "Connections",
        Icon::TriangleAlert,
        None,
    ),
    entry(
        CommandId::Go(Goes::Inbox),
        "go-inbox",
        copy::menus::GO_TO_INBOX,
        Icon::Inbox,
        None,
    ),
    entry(
        CommandId::Go(Goes::Kept),
        "go-kept",
        copy::menus::GO_TO_KEPT,
        Icon::Star,
        None,
    ),
    entry(
        CommandId::Go(Goes::Snoozed),
        "go-snoozed",
        copy::menus::GO_TO_SNOOZED,
        Icon::Clock,
        None,
    ),
    entry(
        CommandId::Go(Goes::Done),
        "go-done",
        copy::menus::GO_TO_DONE,
        Icon::Archive,
        None,
    ),
    entry(
        CommandId::Go(Goes::Trash),
        "go-trash",
        copy::menus::GO_TO_TRASH,
        Icon::Trash,
        None,
    ),
    entry(
        CommandId::Empty(Bin::Trash),
        "empty-trash",
        "Empty Trash\u{2026}",
        Icon::Command,
        None,
    ),
    entry(
        CommandId::Empty(Bin::Spam),
        "empty-spam",
        "Empty Spam\u{2026}",
        Icon::Command,
        None,
    ),
    entry(
        CommandId::TidyUp,
        "tidy-up",
        copy::menus::TIDY_UP,
        Icon::Archive,
        None,
    ),
    entry(
        CommandId::Sidebar,
        "sidebar",
        "Hide sidebar",
        Icon::PanelLeft,
        Some(StandardAction::ToggleSidebar),
    ),
    entry(
        CommandId::AddAccount,
        "add-account",
        "Add account\u{2026}",
        Icon::Plus,
        None,
    ),
    entry(
        CommandId::Import,
        "import",
        "Import mail\u{2026}",
        Icon::Plus,
        None,
    ),
    entry(
        CommandId::Export,
        "export",
        "Export mail\u{2026}",
        Icon::Forward,
        None,
    ),
    // The contact book in and out, as vCard.
    entry(
        CommandId::Contacts(Cards::Import),
        "import-contacts",
        copy::menus::IMPORT_CONTACTS,
        Icon::Group,
        None,
    ),
    entry(
        CommandId::Contacts(Cards::Export),
        "export-contacts",
        copy::menus::EXPORT_CONTACTS,
        Icon::Group,
        None,
    ),
    entry(
        CommandId::NewView,
        "new-view",
        "New view\u{2026}",
        Icon::Search,
        None,
    ),
    entry(
        CommandId::Settings,
        "settings",
        "Settings\u{2026}",
        Icon::Settings,
        None,
    ),
    // Each page of Settings, named so that "set" finds them all.
    entry(
        CommandId::SettingsAt(SettingsPage::General),
        "settings-general",
        "General Settings",
        Icon::Settings,
        None,
    ),
    entry(
        CommandId::SettingsAt(SettingsPage::Accounts),
        "settings-accounts",
        "Accounts Settings",
        Icon::Mail,
        None,
    ),
    entry(
        CommandId::SettingsAt(SettingsPage::Rules),
        "settings-rules",
        "Rules Settings",
        Icon::FolderInput,
        None,
    ),
    entry(
        CommandId::SettingsAt(SettingsPage::Keyboard),
        "settings-keyboard",
        "Keyboard Shortcuts Settings",
        Icon::Keyboard,
        None,
    ),
    entry(
        CommandId::Theme(Look::Light),
        "theme-light",
        "Theme light",
        Icon::Settings,
        None,
    ),
    entry(
        CommandId::Theme(Look::Dark),
        "theme-dark",
        "Theme dark",
        Icon::Settings,
        None,
    ),
    entry(
        CommandId::Theme(Look::System),
        "theme-system",
        "Theme system",
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
#[cfg(test)]
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

/// Whether a Go command already opens the place listing `source`.
pub(in crate::ui) fn gone_to(source: &Source) -> bool {
    ENTRIES.iter().any(|entry| match entry.id {
        CommandId::Go(goes) => goes.source() == *source,
        _ => false,
    })
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
    let mail_id = |hit: &MailHit| format!("mail:{}", hit.summary.id);
    if let Some(top) = &results.top {
        match top {
            Top::Mail(hit) if key == mail_id(hit) => return Some(Pick::Open(hit.summary.id)),
            Top::Person(hit) if key == format!("person:{}", hit.email.to_ascii_lowercase()) => {
                return Some(Pick::From(hit.email.to_ascii_lowercase()));
            }
            Top::Action(hit) => {
                if let Some(entry) = entry_hit(hit)
                    && key == action_key(entry)
                {
                    return Some(Pick::Action(entry.id));
                }
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
    for entry in results.actions.iter().filter_map(entry_hit) {
        if key == action_key(entry) {
            return Some(Pick::Action(entry.id));
        }
    }
    None
}

/// The groups [`rows_of`] files its rows under, which [`super::sections::sections`] regroups
/// by. Not shown: the panel draws its own titles.
pub(in crate::ui) const TOP_HIT: &str = "top";
pub(in crate::ui) const RECENT: &str = "recent";
pub(in crate::ui) const MAIL: &str = "mail";
pub(in crate::ui) const PEOPLE: &str = "people";
pub(in crate::ui) const ACTIONS: &str = "actions";

/// The rows the menu paints for one `search::run`.
pub(in crate::ui) fn rows_of(
    results: &Results,
    names: &HashMap<String, String>,
    query: &str,
    keymap: &Keymap,
) -> Vec<MenuItem> {
    let mut out = Vec::new();
    if let Some(top) = &results.top {
        out.push(top_item(top, names, query, keymap));
    }
    let mail_group = if query.trim().is_empty() {
        RECENT
    } else {
        MAIL
    };
    for hit in &results.mail {
        out.push(mail_item(hit, mail_group, query));
    }
    for hit in &results.people {
        out.push(person_item(hit, names, query, PEOPLE));
    }
    for hit in &results.actions {
        out.push(action_item(hit, ACTIONS, keymap));
    }
    out
}

fn top_item(top: &Top, names: &HashMap<String, String>, query: &str, keymap: &Keymap) -> MenuItem {
    match top {
        Top::Mail(hit) => mail_item(hit, TOP_HIT, query),
        Top::Person(hit) => person_item(hit, names, query, TOP_HIT),
        Top::Action(hit) => action_item(hit, TOP_HIT, keymap),
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
        key: entry.map_or_else(String::new, action_key),
        tile: Tile::Icon(entry.map_or(Icon::Command, |entry| entry.icon)),
        name: hit.command.label.clone(),
        help: None,
        right: shortcut.map(Right::Shortcut).unwrap_or(Right::None),
        group: Some(group.to_owned()),
        marks: hit.indices.clone(),
        title: Vec::new(),
        detail: Vec::new(),
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
