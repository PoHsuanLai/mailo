//! The ⌘K menu over the reference fixture, and the pages its screenshots are taken from.

use super::super::app::App;
use super::items::{Pick, interpret, rows_of, search_now, tokens};
use super::*;
use crate::ui::fixtures::work;
use crate::ui::view::Shell;
use dioxus_core::VirtualDom;
use mail_core::search::{Results, Top};
use mail_domain::Filter;
use std::collections::HashMap;

/// The command menu open on `dana`, as a page a browser can photograph.
#[component]
pub(in crate::ui) fn MenuPicture() -> Element {
    let shell = use_signal(|| Shell {
        command: Some("dana".to_owned()),
        ..Shell::default()
    });
    let pages = use_signal(|| 1u32);
    let revision = use_signal(|| 0u64);
    let side_hidden = use_signal(|| false);
    let spaces = use_signal(crate::ui::space::Spaces::default);
    // Inside a quire root, as the window has it: the palette floats in its overlay.
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            CommandMenu { shell, pages, revision, side_hidden, spaces }
        }
    }
}

/// The open command menu and a label menu, so the stylesheet test sees those classes.
#[component]
pub(in crate::ui) fn OpenMenus() -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let known = mail_core::query::known_labels(&store);
    let shell = use_signal(|| Shell {
        command: Some("dana".to_owned()),
        labels: known,
        ..Shell::default()
    });
    let pages = use_signal(|| 1u32);
    let revision = use_signal(|| 0u64);
    let side_hidden = use_signal(|| false);
    let spaces = use_signal(crate::ui::space::Spaces::default);
    let summary = mail_core::search::Source::listed(
        store.as_ref(),
        &Filter::All,
        mail_core::search::first(1),
        Utc::now(),
    )
    .into_iter()
    .next();
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            CommandMenu { shell, pages, revision, side_hidden, spaces }
            // quire's menu, floating in the same root's overlay.
            if let Some(summary) = summary {
                {
                    let id = summary.id;
                    rsx! { super::super::menus::LabelMenu { id, summary, shell, revision, anchor: None } }
                }
            }
        }
    }
}

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
fn dana_has_a_person_and_a_mail_group() {
    let built = work();
    let (results, names) = at_dana(&built.store);
    let items = rows_of(&results, &names, "dana");
    let groups: Vec<&str> = items
        .iter()
        .filter_map(|item| item.group.as_deref())
        .collect();
    assert!(
        items.iter().any(|item| item.key.starts_with("person:")),
        "no person for dana in {groups:?}"
    );
    assert!(
        items.iter().any(|item| item.key.starts_with("mail:")),
        "no mail for dana in {groups:?}"
    );
    assert!(groups.contains(&"People"), "no People group in {groups:?}");
    assert!(
        groups.contains(&"Mail") || groups.contains(&"Top hit"),
        "no Mail group in {groups:?}"
    );
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

#[test]
fn an_empty_query_shows_actions_and_recent_threads() {
    let built = work();
    let (results, names) = search_now(&built.store, "", Utc::now());
    let items = rows_of(&results, &names, "");
    assert!(
        results
            .actions
            .iter()
            .any(|hit| hit.command.label == "Compose"),
        "actions: {:?}",
        results.actions
    );
    let threads = results.mail.len() + matches!(results.top, Some(Top::Mail(_))) as usize;
    assert!(threads > 0, "no recent threads");
    assert!(
        items
            .iter()
            .any(|item| item.group.as_deref() == Some("Actions")),
        "no Actions group"
    );
    assert!(
        items
            .iter()
            .any(|item| item.group.as_deref() == Some("Recent") || item.key.starts_with("mail:")),
        "no recent rows"
    );
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

/// The People row's name is drawn with the typed characters as `<mark>` nodes: quire's palette
/// row, whose title is the runs mailo marked.
#[tokio::test]
async fn dana_is_marked_in_the_persons_name() {
    let built = work();
    let mut menus = VirtualDom::new(MenuPicture).with_root_context(built.store);
    menus.rebuild_in_place();
    crate::ui::fixtures::drain(&mut menus);
    let page = dioxus_ssr::render(&menus);
    let people = page.find(">People<").expect("no People group on dana");
    let after = &page[people..];
    let name = &after[after
        .find("<b class=\"ds-row-title")
        .expect("a person item has a name")..];
    let name = &name[..name.find("</b>").expect("the name closes")];
    assert!(
        name.contains("<mark class=\"ds-mark\">Dana</mark>"),
        "the person's name marks nothing: {name}"
    );
}

/// The command menu open on `dana` (`menus.html`), and the label menu open on the second row
/// (`menus-label.html`), each in both themes, as pages a browser can photograph. Two pages
/// because two open menus at once is a state the window never shows.
#[tokio::test]
#[ignore]
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
        dom.rebuild_in_place();
        settle(&mut dom).await;
        // ⌘K in the window, then "dana" in quire's palette, which floats in the root's
        // overlay and answers once the field has been still.
        crate::ui::fixtures::chord(
            &mut dom,
            "t",
            dioxus::html::input_data::keyboard_types::Modifiers::CONTROL,
            dioxus_core::ElementId(crate::ui::fixtures::INSIDE_THE_SHELL as usize),
        );
        let opened = crate::ui::fixtures::drain_seen(&mut dom);
        // The card and its field are both named for what they are; the field is drawn last.
        let field = *opened
            .all("aria-label", "Search and commands")
            .last()
            .expect("the palette's field");
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
