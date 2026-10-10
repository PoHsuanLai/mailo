//! The search bar's rows over the reference fixture, what its commands do, and the pages its
//! screenshots are taken from. How the bar itself behaves is `bar_tests.rs`.

use super::super::app::App;
use super::items::{Pick, interpret, rows_of, search_now, tokens};
use super::*;
use crate::ui::fixtures::work;
use crate::ui::view::Shell;
use chrono::Utc;
use dioxus_core::VirtualDom;
use ds::prelude::*;
use mail_core::search::{Results, Top};
use std::collections::HashMap;

fn at_dana(store: &SqliteStore) -> (Results, HashMap<String, String>) {
    search_now(store, "dana", Utc::now())
}

/// The addresses the ⌘K menu's person rows name for `query`, top hit first, as drawn.
pub(in crate::ui) fn people_for(store: &SqliteStore, query: &str) -> Vec<String> {
    let (results, names) = search_now(store, query, Utc::now());
    rows_of(&results, &names, query)
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
    let items = rows_of(&results, &names, "");
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
    let items = rows_of(&results, &names, "sidebar");
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
    let person = rows_of(&results, &names, "dana")
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

#[test]
fn the_entries_that_are_a_page_of_settings_name_it() {
    use crate::ui::view::SettingsPage;
    // The entries that do name a page are run end to end in
    // `each_settings_entry_opens_the_settings_window_on_its_page`.
    const CASES: &[(&str, Option<SettingsPage>)] = &[("Settings…", None), ("Add account…", None)];
    for (label, page) in CASES {
        assert_eq!(settings_page_of(label), *page, "{label:?}");
    }
    let offered: Vec<String> = super::items::commands()
        .into_iter()
        .map(|command| command.label)
        .collect();
    for (label, _) in CASES {
        assert!(
            offered.iter().any(|one| one == label),
            "{label:?} is not offered"
        );
    }
}

/// A button that runs the command `label`, as Enter on its row in the menu does.
#[component]
fn Runs(label: String) -> Element {
    let shell = use_signal(|| Shell {
        bar: Bar::Open(crate::ui::view::BarOpen::over(String::new())),
        ..Shell::default()
    });
    let pages = use_signal(|| 1u32);
    let mut revision = use_signal(|| 0u64);
    let mut side_hidden = use_signal(|| false);
    let spaces = use_signal(|| crate::ui::space::first_run(&[]));
    let name = format!("run {label}");
    rsx! {
        button {
            "aria-label": name,
            onclick: move |_| run_action(shell, pages, &mut revision, &mut side_hidden, spaces, &label),
        }
    }
}

#[tokio::test]
async fn each_settings_entry_opens_the_settings_window_on_its_page() {
    use crate::ui::settings_window::SettingsWindows;
    use crate::ui::settings_window::tests::Asked;
    use crate::ui::view::SettingsPage;
    let offered: Vec<String> = super::items::commands()
        .into_iter()
        .map(|command| command.label)
        .collect();
    for (label, page) in [
        ("General Settings", SettingsPage::General),
        ("Accounts Settings", SettingsPage::Accounts),
        ("Contacts Settings", SettingsPage::Contacts),
        ("Rules Settings", SettingsPage::Rules),
        ("Keys and Certificates Settings", SettingsPage::Keys),
        ("Keyboard Shortcuts Settings", SettingsPage::Keyboard),
    ] {
        assert!(
            offered.iter().any(|one| one == label),
            "{label:?} is not offered"
        );
        crate::ui::fixtures::dispatching();
        let asked = Arc::new(Asked::default());
        let mut dom = VirtualDom::new_with_props(
            Runs,
            RunsProps {
                label: label.to_owned(),
            },
        )
        .with_root_context(SettingsWindows(asked.clone()));
        let seen = crate::ui::fixtures::rebuild_into(&mut dom);
        crate::ui::fixtures::click(&mut dom, seen.one("aria-label", &format!("run {label}")));
        assert_eq!(
            asked.asks(),
            [Some(crate::ui::settings_window::SettingsAt::Page(page))],
            "{label:?}"
        );
    }
}
