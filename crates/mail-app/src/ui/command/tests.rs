//! The search bar's rows over the reference fixture, what its commands do, and the pages its
//! screenshots are taken from. How the bar itself behaves is `bar_tests.rs`.

use super::super::app::App;
use super::items::{CommandId, Goes, Look, Pick, interpret, rows_of, search_now, tokens};
use super::*;
use crate::ui::bin::Bin;
use crate::ui::fixtures::work;
use crate::ui::menu::Right;
use crate::ui::view::SettingsPage;
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

#[test]
fn enter_on_the_top_hit_opens_that_thread() {
    let built = work();
    let (results, _) = at_dana(&built.store);
    let Top::Mail(hit) = results.top.as_ref().expect("dana has a top hit") else {
        panic!("the top hit is not the thread, got {:?}", results.top);
    };
    let mut shell = Shell::default();
    let key = format!("mail:{}", hit.summary.id);
    match interpret(&results, &key) {
        Some(Pick::Open(id)) => shell.open(id),
        other => panic!("enter did not open the thread: {other:?}"),
    }
    assert_eq!(shell.open, Some(hit.summary.id));
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
        ("pinned", Shown::Visible, "Hide sidebar"),
        ("hidden", Shown::Hidden, "Show sidebar"),
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
    assert_eq!(renamed.name, "Show sidebar");
    let marked: String = renamed
        .name
        .chars()
        .enumerate()
        .filter(|(index, _)| renamed.marks.contains(&(*index as u32)))
        .map(|(_, c)| c)
        .collect();
    assert_eq!(marked, "sidebar");
}

#[test]
fn from_dana_is_a_chip() {
    assert_eq!(tokens("from:dana spec"), vec!["from:dana".to_owned()]);
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

/// The search bar open on `dana` (`menus.html`), and the label menu open on the second row
/// (`menus-label.html`), each in both themes, as pages a browser can photograph. Two pages
/// because two open menus at once is a state the window never shows.
#[tokio::test]
#[ignore = "writes the menus pages to a file for screenshots"]
async fn render_the_menus_to_a_file() {
    use crate::ui::fixtures::{dispatching, rebuild_into};

    use crate::ui::fixtures::in_scheme;

    dispatching();
    // Rendered once per scheme, so the Space's tint and hue are the ones each scheme derives.
    for (suffix, scheme) in [("", Scheme::Light), ("-dark", Scheme::Dark)] {
        let built = work();
        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store.clone())
            .with_root_context(built.dirs.clone())
            .with_root_context(in_scheme(scheme));
        // ⌘K in the window, then "dana" in the bar, whose panel floats in the root's overlay
        // and answers once the field has been still.
        let seen = crate::ui::fixtures::rebuild_into(&mut dom);
        crate::ui::fixtures::chord(
            &mut dom,
            "k",
            crate::ui::fixtures::PRIMARY,
            dioxus_core::ElementId(crate::ui::fixtures::INSIDE_THE_SHELL as usize),
        );
        let field = *seen
            .all("aria-label", LABEL)
            .last()
            .expect("the bar's field");
        crate::ui::fixtures::type_into(&mut dom, field, "dana");
        for _ in 0..6 {
            settle(&mut dom).await;
        }
        crate::ui::fixtures::drain(&mut dom);
        let command = dioxus_ssr::render(&dom);

        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store.clone())
            .with_root_context(built.dirs)
            .with_root_context(in_scheme(scheme));
        let seen = rebuild_into(&mut dom);
        let second = crate::ui::fixtures::listed_subjects(&dioxus_ssr::render(&dom))
            .get(1)
            .cloned()
            .expect("a second row");
        crate::ui::fixtures::row_action(&mut dom, &seen, &second, "Label…").await;
        settle(&mut dom).await;
        let label = dioxus_ssr::render(&dom);
        // The label menu is quire's, floating in the root's overlay where the second row's menu
        // stood.
        assert!(
            label.contains("role=\"listbox\" aria-label=\"Labels\""),
            "the label menu did not open"
        );

        for (name, body) in [("menus", &command), ("menus-label", &label)] {
            crate::ui::fixtures::write_page(
                &format!("{name}{suffix}"),
                &crate::ui::fixtures::page(body, ""),
            );
        }
    }
}

async fn settle(dom: &mut VirtualDom) {
    tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
        .await
        .ok();
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
}

/// Every command the panel offers, by the id it is dispatched on and the key its row carries.
/// A command added to the panel is added here, and a label renamed changes nothing in this
/// table.
const EVERY_COMMAND: &[(CommandId, &str)] = &[
    (CommandId::Compose, "action:compose"),
    (CommandId::FromTemplate, "action:from-template"),
    (CommandId::ForwardAttached, "action:forward-attached"),
    (CommandId::Print, "action:print"),
    (CommandId::Sync, "action:sync"),
    (CommandId::Connections, "action:connections"),
    (CommandId::Go(Goes::Inbox), "action:go-inbox"),
    (CommandId::Go(Goes::Kept), "action:go-kept"),
    (CommandId::Go(Goes::Snoozed), "action:go-snoozed"),
    (CommandId::Go(Goes::Done), "action:go-done"),
    (CommandId::Go(Goes::Trash), "action:go-trash"),
    (CommandId::Empty(Bin::Trash), "action:empty-trash"),
    (CommandId::Empty(Bin::Spam), "action:empty-spam"),
    (CommandId::TidyUp, "action:tidy-up"),
    (CommandId::Sidebar, "action:sidebar"),
    (CommandId::AddAccount, "action:add-account"),
    (CommandId::Import, "action:import"),
    (CommandId::Export, "action:export"),
    (CommandId::Contacts(Cards::Import), "action:import-contacts"),
    (CommandId::Contacts(Cards::Export), "action:export-contacts"),
    (CommandId::NewView, "action:new-view"),
    (CommandId::Settings, "action:settings"),
    (
        CommandId::SettingsAt(SettingsPage::General),
        "action:settings-general",
    ),
    (
        CommandId::SettingsAt(SettingsPage::Accounts),
        "action:settings-accounts",
    ),
    (
        CommandId::SettingsAt(SettingsPage::Rules),
        "action:settings-rules",
    ),
    (
        CommandId::SettingsAt(SettingsPage::Keyboard),
        "action:settings-keyboard",
    ),
    (CommandId::Theme(Look::Light), "action:theme-light"),
    (CommandId::Theme(Look::Dark), "action:theme-dark"),
    (CommandId::Theme(Look::System), "action:theme-system"),
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

/// A Go command finds its place by what the place lists, so each one names a place the
/// window has.
#[test]
fn each_go_command_finds_its_place() {
    let shell = Shell::default();
    for (id, key) in EVERY_COMMAND {
        let CommandId::Go(goes) = id else {
            continue;
        };
        assert!(
            shell
                .places
                .iter()
                .any(|place| place.source == goes.source()),
            "{key} goes nowhere"
        );
        assert!(
            super::items::gone_to(&goes.source()),
            "{key} is not offered"
        );
    }
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
    let spaces = use_signal(|| crate::ui::space::first_run(&[]));
    // Named by its id, not a fixed word: the fixture sees only attributes the template fills in.
    let name = format!("run {id:?}");
    rsx! {
        button {
            "aria-label": name,
            onclick: move |_| run_action(shell, pages, &mut revision, &mut side_hidden, spaces, id),
        }
    }
}

#[tokio::test]
async fn each_settings_entry_opens_the_settings_window_on_its_page() {
    use crate::ui::settings_window::SettingsWindows;
    use crate::ui::settings_window::tests::Asked;
    for page in [
        SettingsPage::General,
        SettingsPage::Accounts,
        SettingsPage::Rules,
        SettingsPage::Keyboard,
    ] {
        let id = CommandId::SettingsAt(page);
        crate::ui::fixtures::dispatching();
        let asked = Arc::new(Asked::default());
        let mut dom = VirtualDom::new_with_props(Runs, RunsProps { id })
            .with_root_context(SettingsWindows(asked.clone()));
        let seen = crate::ui::fixtures::rebuild_into(&mut dom);
        crate::ui::fixtures::click(&mut dom, seen.one("aria-label", &format!("run {id:?}")));
        assert_eq!(
            asked.asks(),
            [Some(crate::ui::settings_window::SettingsAt::Page(page))],
            "{id:?}"
        );
    }
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
