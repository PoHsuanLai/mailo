//! The search bar's rows over the reference fixture, what its commands do, and the pages its
//! screenshots are taken from. How the bar itself behaves is `bar_tests.rs`.

use super::items::{CommandId, Pick, chips, complete_syntax, interpret, rows_of, search_now};
use super::*;
use crate::ui::bin::Bin;
use crate::ui::fixtures::work;
use crate::ui::menu::Right;
use crate::ui::view::Shell;
use chordkit::Platform;
use chrono::Utc;
use dioxus_core::VirtualDom;
use ds::prelude::*;
use mail_core::search::{Results, Top};
use std::collections::HashMap;

/// `platform`'s conventions as a window's keymap, for the hints beside the commands.
fn conventional(platform: Platform) -> chordkit::Keymap {
    chordkit::Keymap::conventional(platform)
}

fn at_dana(store: &SqliteStore) -> (Results, HashMap<String, String>) {
    search_now(store, "dana", Utc::now())
}

/// The addresses the ⌘K menu's person rows name for `query`, top hit first, as drawn.
pub(in crate::ui) fn people_for(store: &SqliteStore, query: &str) -> Vec<String> {
    let (results, names) = search_now(store, query, Utc::now());
    rows_of(&results, &names, query, &conventional(Platform::MacOs))
        .into_iter()
        .filter_map(|item| item.key.strip_prefix("person:").map(str::to_owned))
        .collect()
}

#[test]
fn a_person_the_ranker_put_on_top_is_the_books_best_match() {
    // Nobody but the book says who "da" is: the menu's people are its answer, in its order.
    let (store, _dir) = crate::ui::contacts::tests::the_book();
    let drawn = people_for(&store, "da");
    let book: Vec<String> = crate::ui::contacts::book::suggest(store.as_ref(), "da")
        .into_iter()
        .map(|person| person.address)
        .collect();
    assert!(!drawn.is_empty(), "no people for da");
    assert_eq!(drawn, book[..drawn.len()], "the menu and the book disagree");
    // Operators alone name nobody.
    assert_eq!(people_for(&store, "from:dana"), Vec::<String>::new());
}

/// Typed text is the list's search: the panel's rows are people and commands, never mail, even
/// where the ranker's top hit was a conversation.
#[test]
fn the_panel_draws_no_mail_of_its_own() {
    let built = work();
    let (results, names) = at_dana(&built.store);
    assert!(
        matches!(results.top, Some(Top::Mail(_))) || !results.mail.is_empty(),
        "dana finds no mail in the fixture"
    );
    let rows = rows_of(&results, &names, "dana", &conventional(Platform::MacOs));
    assert!(
        rows.iter()
            .all(|row| row.key.starts_with("person:") || row.key.starts_with("action:")),
        "{:?}",
        rows.iter().map(|row| &row.key).collect::<Vec<_>>()
    );
}

/// The sidebar's row reads as what picking it does: "Hide sidebar" while it is pinned, "Show
/// sidebar" while it is hidden, under one key either way, and no other row is touched.
#[test]
fn the_sidebar_row_names_what_a_pick_does() {
    use super::items::{SIDEBAR_KEY, restate_sidebar};
    let built = work();
    let (results, names) = search_now(&built.store, "", Utc::now());
    let items = rows_of(&results, &names, "", &conventional(Platform::MacOs));
    let wording = |sidebar: Shown, query: &str| -> Vec<(String, String)> {
        items
            .iter()
            .cloned()
            .map(|item| restate_sidebar(item, sidebar, query))
            .map(|item| (item.key, item.name))
            .collect()
    };
    let at = |rows: &[(String, String)], key: &str| {
        rows.iter()
            .find(|(k, _)| k == key)
            .map(|(_, name)| name.clone())
    };
    const CASES: &[(&str, Shown, &str)] = &[
        (
            "pinned",
            Shown::Visible,
            crate::ui::copy::menus::HIDE_SIDEBAR,
        ),
        (
            "hidden",
            Shown::Hidden,
            crate::ui::copy::menus::SHOW_SIDEBAR,
        ),
    ];
    for (name, sidebar, want) in CASES {
        let rows = wording(*sidebar, "");
        assert_eq!(at(&rows, SIDEBAR_KEY).as_deref(), Some(*want), "{name}");
    }
    // Every other row keeps its name.
    let (pinned, hidden) = (wording(Shown::Visible, ""), wording(Shown::Hidden, ""));
    let others = |rows: &[(String, String)]| -> Vec<(String, String)> {
        rows.iter()
            .filter(|(k, _)| k != SIDEBAR_KEY)
            .cloned()
            .collect()
    };
    assert_eq!(others(&pinned), others(&hidden));
}

#[test]
fn the_renamed_sidebar_row_is_marked_for_its_own_wording() {
    use super::items::{SIDEBAR_KEY, restate_sidebar};
    let built = work();
    let (results, names) = search_now(&built.store, "sidebar", Utc::now());
    let items = rows_of(&results, &names, "sidebar", &conventional(Platform::MacOs));
    let row = items
        .into_iter()
        .find(|item| item.key == SIDEBAR_KEY)
        .expect("the sidebar row matches");
    let renamed = restate_sidebar(row, Shown::Hidden, "sidebar");
    assert_eq!(renamed.name, crate::ui::copy::menus::SHOW_SIDEBAR);
    let marked: String = renamed
        .name
        .chars()
        .enumerate()
        .filter(|(index, _)| renamed.marks.contains(&(*index as u32)))
        .map(|(_, c)| c)
        .collect();
    assert_eq!(marked, crate::ui::copy::aria::SIDEBAR);
}

/// The chips under the field: the syntax while nothing is typed or while the last word starts an
/// operator, which Tab completes; otherwise the operators the search holds.
#[test]
fn the_chips_offer_the_syntax_until_a_search_holds_operators() {
    let syntax: Vec<String> = super::items::SYNTAX
        .iter()
        .map(|chip| (*chip).to_owned())
        .collect();
    const CASES: &[(&str, &[&str])] = &[
        ("lunch fr", &["from:"]),
        ("is:", &["is:unread", "is:starred"]),
        ("lunch is:u", &["is:unread"]),
        ("from:dana spec", &["from:dana"]),
        ("lunch ", &[]),
        ("from:", &[]),
    ];
    assert_eq!(chips(""), syntax);
    assert_eq!(chips("   "), syntax);
    for (query, want) in CASES {
        assert_eq!(chips(query), *want, "{query:?}");
    }
    assert_eq!(complete_syntax("lunch fr").as_deref(), Some("lunch from:"));
    assert_eq!(complete_syntax("is:u").as_deref(), Some("is:unread"));
    assert_eq!(complete_syntax("lunch"), None);
}

/// The People row's name keeps the typed characters marked, as the ⌘K menu drew them. quire's
/// `Menu` draws no marks in a title yet, so they are data the panel holds, not markup.
#[test]
fn dana_is_marked_in_the_persons_name() {
    let built = work();
    let (results, names) = at_dana(&built.store);
    let person = rows_of(&results, &names, "dana", &conventional(Platform::MacOs))
        .into_iter()
        .find(|item| item.key.starts_with("person:"))
        .expect("a person for dana");
    let name = &person.title[0];
    let marked: String = name
        .text
        .chars()
        .enumerate()
        .filter(|(index, _)| name.marks.contains(&(*index as u32)))
        .map(|(_, c)| c)
        .collect();
    assert_eq!(marked, "Dana", "the person's name marks {marked:?}");
}

/// Every command the panel offers, by the id it is dispatched on and the key its row carries.
/// A command added to the panel is added here, and a label renamed changes nothing in this
/// table.
const EVERY_COMMAND: &[(CommandId, &str)] = &[
    (CommandId::Compose, "action:compose"),
    (CommandId::FromTemplate, "action:from-template"),
    (CommandId::ForwardAttached, "action:forward-attached"),
    (CommandId::ReplyList, "action:reply-list"),
    (CommandId::Redirect, "action:redirect"),
    (CommandId::Again, "action:edit-as-new"),
    (CommandId::ApplyRules, "action:apply-rules"),
    (CommandId::Print, "action:print"),
    (CommandId::Sync, "action:sync"),
    (CommandId::Empty(Bin::Trash), "action:empty-trash"),
    (CommandId::Empty(Bin::Spam), "action:empty-spam"),
    (CommandId::TidyUp, "action:tidy-up"),
    (CommandId::Sidebar, "action:sidebar"),
    (CommandId::Conversations, "action:conversations"),
    (CommandId::AddAccount, "action:add-account"),
    (CommandId::Import, "action:import"),
    (CommandId::Export, "action:export"),
    (CommandId::Contacts(Cards::Import), "action:import-contacts"),
    (CommandId::Contacts(Cards::Export), "action:export-contacts"),
    (CommandId::NewView, "action:new-view"),
    (CommandId::Settings, "action:settings"),
];

/// Each row the panel draws for a command carries its id's key, and that key reads back as
/// that id, whichever way the row is worded: as one of the Actions and as the top hit.
#[test]
fn every_palette_row_is_run_by_its_id() {
    use super::items::{ENTRIES, action_key};
    use mail_core::search::{ActionHit, Command};
    let listed: Vec<(CommandId, String)> = ENTRIES
        .iter()
        .map(|entry| (entry.id, action_key(entry)))
        .collect();
    let want: Vec<(CommandId, String)> = EVERY_COMMAND
        .iter()
        .map(|(id, key)| (*id, (*key).to_owned()))
        .collect();
    assert_eq!(listed, want, "the panel's commands and this table differ");
    let keymap = conventional(Platform::MacOs);
    for (id, key) in EVERY_COMMAND {
        let hit = ActionHit {
            command: Command {
                label: super::items::entry_of(*id).label.to_owned(),
            },
            score: 0,
            indices: Vec::new(),
        };
        let cases = [
            (
                "an action",
                Results {
                    actions: vec![hit.clone()],
                    ..Results::default()
                },
            ),
            (
                "the top hit",
                Results {
                    top: Some(Top::Action(hit.clone())),
                    ..Results::default()
                },
            ),
        ];
        for (name, results) in cases {
            let rows = rows_of(&results, &HashMap::new(), "", &keymap);
            assert_eq!(
                rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
                [*key],
                "{key} as {name}"
            );
            assert_eq!(
                interpret(&results, key),
                Some(Pick::Action(*id)),
                "{key} as {name}"
            );
        }
    }
}

/// Two commands with one label would be one row the matcher cannot tell apart.
#[test]
fn no_two_commands_share_a_label() {
    let labels: Vec<&str> = super::items::ENTRIES
        .iter()
        .map(|entry| entry.label)
        .collect();
    let mut sorted = labels.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), labels.len(), "{labels:?}");
}

/// A button that runs the command `id`, as Enter on its row in the menu does.
#[component]
fn Runs(id: CommandId) -> Element {
    let shell = use_signal(|| Shell {
        bar: Bar::Open(crate::ui::view::BarOpen::over(String::new())),
        ..Shell::default()
    });
    let pages = use_signal(|| 1u32);
    let mut revision = use_signal(|| 0u64);
    let mut side_hidden = use_signal(|| false);
    // Named by its id, not a fixed word: the fixture sees only attributes the template fills in.
    let name = format!("run {id:?}");
    rsx! {
        button {
            "aria-label": name,
            onclick: move |_| run_action(shell, pages, &mut revision, &mut side_hidden, id),
        }
    }
}

/// Settings… is the one row for Settings: it opens the window, which keeps its own pages.
#[tokio::test]
async fn settings_opens_the_settings_window() {
    use crate::ui::settings_window::SettingsWindows;
    use crate::ui::settings_window::tests::Asked;
    let id = CommandId::Settings;
    crate::ui::fixtures::dispatching();
    let asked = Arc::new(Asked::default());
    let mut dom = VirtualDom::new_with_props(Runs, RunsProps { id })
        .with_root_context(SettingsWindows(asked.clone()));
    let seen = crate::ui::fixtures::rebuild_into(&mut dom);
    crate::ui::fixtures::click(&mut dom, seen.one("aria-label", &format!("run {id:?}")));
    assert_eq!(asked.asks().len(), 1, "{:?}", asked.asks());
}

/// The chord beside a command is the one the keymap binds, in the platform's own words: a Mac
/// draws glyphs, Windows draws Ctrl and its own key for the sidebar.
#[test]
fn a_command_s_hint_is_drawn_for_the_platform() {
    let built = work();
    let (results, names) = search_now(&built.store, "", Utc::now());
    let hint = |platform: Platform, key: &str| -> Option<String> {
        rows_of(&results, &names, "", &conventional(platform))
            .into_iter()
            .find(|item| item.key == key)
            .and_then(|item| match item.right {
                Right::Shortcut(text) => Some(text),
                _ => None,
            })
    };
    const CASES: &[(&str, &str, &str)] = &[
        ("action:compose", "\u{2318}N", "Ctrl+N"),
        ("action:print", "\u{2318}P", "Ctrl+P"),
        ("action:sidebar", "\u{2303}\u{2318}S", "F9"),
    ];
    for (key, mac, windows) in CASES {
        assert_eq!(
            hint(Platform::MacOs, key).as_deref(),
            Some(*mac),
            "{key} on a Mac"
        );
        assert_eq!(
            hint(Platform::Windows, key).as_deref(),
            Some(*windows),
            "{key} on Windows"
        );
    }
}
