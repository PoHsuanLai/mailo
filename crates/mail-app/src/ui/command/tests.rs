//! The Ctrl T menu over the reference fixture, and the pages its screenshots are taken from.

use super::super::app::App;
use super::items::{Pick, interpret, rows_of, search_now, tokens};
use super::*;
use crate::search::{Results, Top};
use crate::ui::fixtures::work;
use crate::view::Shell;
use crate::view::Theme;
use dioxus_core::VirtualDom;
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
    let sync_state = use_signal(|| crate::view::SyncState::Idle);
    let spaces = use_signal(crate::space::Spaces::default);
    let in_a_field = use_signal(|| false);
    rsx! {
        CommandMenu { shell, pages, revision, side_hidden, sync_state, spaces, in_a_field }
    }
}

/// The open command menu and a label menu, so the stylesheet test sees those classes.
#[component]
pub(in crate::ui) fn OpenMenus() -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let known = crate::query::known_labels(&store);
    let shell = use_signal(|| Shell {
        command: Some("dana".to_owned()),
        labels: known,
        ..Shell::default()
    });
    let pages = use_signal(|| 1u32);
    let revision = use_signal(|| 0u64);
    let side_hidden = use_signal(|| false);
    let sync_state = use_signal(|| crate::view::SyncState::Idle);
    let spaces = use_signal(crate::space::Spaces::default);
    let in_a_field = use_signal(|| false);
    let summary = crate::search::Source::ranked(store.as_ref(), &Filter::All, 1, Utc::now())
        .into_iter()
        .next()
        .map(|(summary, _)| summary);
    rsx! {
        CommandMenu { shell, pages, revision, side_hidden, sync_state, spaces, in_a_field }
        if let Some(summary) = summary {
            {
                let id = summary.id;
                rsx! { super::super::menus::LabelMenu { id, summary, shell, revision } }
            }
        }
        Menu {
            title: "Empty".to_owned(),
            items: Vec::new(),
            filterable: false,
            on_pick: move |_| {},
            on_close: move |_| {},
            on_query: move |_| {},
            slim: true,
            active: None,
        }
        li { class: "list-g", "Today" }
    }
}

fn at_dana(store: &SqliteStore) -> (Results, HashMap<String, String>) {
    search_now(store, "dana", Utc::now())
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

#[test]
fn from_dana_is_a_chip() {
    assert_eq!(tokens("from:dana spec"), vec!["from:dana".to_owned()]);
}

/// The People row's name is drawn with the typed characters as `<mark>` nodes.
#[test]
fn dana_is_marked_in_the_persons_name() {
    let built = work();
    let mut menus = VirtualDom::new(MenuPicture).with_root_context(built.store);
    menus.rebuild_in_place();
    let page = dioxus_ssr::render(&menus);
    let people = page.find(">People<").expect("no People group on dana");
    let after = &page[people..];
    let name = &after[after.find("<b>").expect("a person item has a name")..];
    let name = &name[..name.find("</b>").expect("the name closes")];
    assert!(
        name.contains("<mark>Dana</mark>"),
        "the person's name marks nothing: {name}"
    );
}

/// The command menu open on `dana` (`menus.html`), and the label menu open on the second row
/// (`menus-label.html`), each in both themes, as pages a browser can photograph. Two pages
/// because two open menus at once is a state the window never shows.
#[tokio::test]
#[ignore]
async fn render_the_menus_to_a_file() {
    use crate::ui::fixtures::{click, dispatching, rebuild_into};

    dispatching();
    let built = work();
    let space = crate::space::load(&built.dirs.config).current_space();

    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    dom.rebuild_in_place();
    settle(&mut dom).await;
    let mut menus = VirtualDom::new(MenuPicture).with_root_context(built.store.clone());
    menus.rebuild_in_place();
    let command = inject(&dioxus_ssr::render(&dom), &dioxus_ssr::render(&menus));

    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs);
    let seen = rebuild_into(&mut dom);
    let buttons = seen.all("aria-label", "Label");
    let second = *buttons.get(1).expect("a second row with a Label button");
    click(&mut dom, second);
    settle(&mut dom).await;
    let label = dioxus_ssr::render(&dom);
    let rows: Vec<&str> = label.split("<li class=\"row\"").collect();
    assert!(
        rows.get(2)
            .is_some_and(|row| row.contains("class=\"row-menu\"")),
        "the label menu is not inside the second row"
    );

    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    for (name, body) in [("menus", &command), ("menus-label", &label)] {
        for (suffix, theme) in [("", Theme::Light), ("-dark", Theme::Dark)] {
            let out = target.join(format!("{name}{suffix}.html"));
            std::fs::write(&out, page(body, theme, &space)).unwrap();
            println!("wrote {}", out.display());
        }
    }
}

async fn settle(dom: &mut VirtualDom) {
    tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
        .await
        .ok();
    dom.render_immediate(&mut dioxus_core::NoOpMutations);
}

fn page(body: &str, theme: Theme, space: &crate::space::Space) -> String {
    use super::super::paint::appearance_script;
    use super::super::style::STYLE;
    let script = appearance_script(&crate::space::Space {
        theme,
        ..space.clone()
    });
    let theme_attr = theme
        .attribute()
        .map(|name| format!(" data-theme=\"{name}\""))
        .unwrap_or_default();
    format!(
        "<!doctype html>\n<html lang=\"en\"{theme_attr}>\
         <head><meta charset=\"utf-8\"><style>{STYLE}</style><script>{script}</script></head>\
         <body>{body}</body></html>\n"
    )
}

/// `extra` as the first child of `.app`, where the window mounts the overlay.
fn inject(page: &str, extra: &str) -> String {
    let Some(at) = page.find("class=\"app\"") else {
        return page.to_owned() + extra;
    };
    let Some(rel) = page[at..].find('>') else {
        return page.to_owned() + extra;
    };
    let close = at + rel;
    format!("{}{extra}{}", &page[..=close], &page[close + 1..])
}
