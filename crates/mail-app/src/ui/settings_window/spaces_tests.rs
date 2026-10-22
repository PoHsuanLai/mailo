//! Settings › Spaces over the reference fixture: the list of Spaces, a Space's page pushed from
//! its row, each of the page's edits written to the Spaces file the other windows read, and a
//! Space moved in the order from its row's right click.

use super::super::tests::opened_on;
use crate::ui::copy;
use crate::ui::fixtures::{
    Seen, Work, click, drain_seen, menu_names, pick_named, right_click, settle_quiet_seen, work,
};
use crate::ui::space::{Spaces, stored};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::VirtualDom;
use ds::style::appearance::theme::Theme;
use ds::style::space::presets::PRESETS;

/// The Spaces as the file holds them now.
fn on_disk(built: &Work) -> Spaces {
    stored(&built.dirs).expect("the Spaces were written")
}

/// The Settings window on the Spaces page, with the first Space's page pushed.
fn first_space(built: &Work) -> (VirtualDom, crate::ui::fixtures::Seen) {
    let (mut dom, seen) = opened_on(built, SettingsPage::Spaces);
    let page = dioxus_ssr::render(&dom);
    let at = page.find("spaces-pane-open-").expect("a Space's row");
    let id: String = page[at..].chars().take_while(|c| *c != '"').collect();
    let seen = click(&mut dom, seen.one("id", &id)).merge(drain_seen(&mut dom));
    (dom, seen)
}

#[test]
fn the_list_names_each_space_and_offers_a_new_one() {
    let built = work();
    let (dom, _) = opened_on(&built, SettingsPage::Spaces);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(">Work<"), "{page}");
    assert!(page.contains(copy::controls::NEW_SPACE), "{page}");
    assert!(
        page.contains("ds-space-swatch"),
        "each Space wears its colour: {page}"
    );
}

#[test]
fn a_space_s_page_has_its_name_colour_theme_and_accounts_and_no_delete_when_it_is_the_last() {
    let built = work();
    let (dom, _) = first_space(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("value=\"Work\""), "the name's field: {page}");
    assert_eq!(
        page.matches("class=\"ds-preset\"").count(),
        PRESETS.len(),
        "{page}"
    );
    assert!(
        page.contains("aria-label=\"Custom\""),
        "Custom, after the presets: {page}"
    );
    assert!(page.contains(copy::fields::APPEARANCE), "{page}");
    assert!(page.contains(copy::aria::ACCOUNTS_IN_SPACE), "{page}");
    assert!(
        !page.contains(copy::controls::DELETE_SPACE),
        "the last Space offers Delete: {page}"
    );
}

#[test]
fn a_colour_and_a_theme_picked_are_written() {
    let built = work();
    let (mut dom, seen) = first_space(&built);
    click(&mut dom, seen.one("aria-label", PRESETS[2].name));
    let after = on_disk(&built);
    assert_eq!(
        after.current().look.dots,
        PRESETS[2].dots,
        "the colour was not kept"
    );
    let seen = drain_seen(&mut dom).merge(seen);
    // System, Light, Dark.
    click(
        &mut dom,
        seen.after("aria-label", copy::fields::APPEARANCE, "aria-pressed")[2],
    );
    assert_eq!(
        on_disk(&built).current().look.theme,
        Theme::Dark,
        "the theme was not kept"
    );
}

#[test]
fn a_new_space_opens_its_page_and_can_be_deleted_after_asking() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::Spaces);
    let before = dioxus_ssr::render(&dom)
        .matches("spaces-pane-open-")
        .count();
    let seen = click(&mut dom, seen.one("aria-label", copy::controls::NEW_SPACE))
        .merge(drain_seen(&mut dom));
    assert_eq!(on_disk(&built).count(), before + 1, "no Space was added");
    let page = dioxus_ssr::render(&dom);
    let name = format!("Space {}", before + 1);
    assert!(
        page.contains(&format!("value=\"{name}\"")),
        "its page: {page}"
    );
    let delete = seen.one("aria-label", &copy::confirm::delete_space(&name));
    let seen = click(&mut dom, delete).merge(drain_seen(&mut dom));
    assert_eq!(on_disk(&built).count(), before + 1, "deleted before asking");
    assert!(
        dioxus_ssr::render(&dom).contains(copy::confirm::DELETE_SPACE_KEEPS),
        "no question"
    );
    // The alert's destructive button, drawn after the page's own.
    let confirm = *seen
        .all("data-role", "destructive")
        .last()
        .expect("the alert's Delete");
    click(&mut dom, confirm);
    assert_eq!(on_disk(&built).count(), before, "the Space stayed");
    assert!(
        dioxus_ssr::render(&dom).contains(copy::controls::NEW_SPACE),
        "the pane went back to the list"
    );
}

#[test]
fn the_window_opens_on_the_space_s_page_it_is_asked_for() {
    let built = work();
    let first = on_disk(&built).current().id;
    let asked = crate::ui::settings_window::SettingsAsked::default();
    asked.ask(crate::ui::settings_window::SettingsAt::Space(first));
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new(crate::ui::settings_window::settings_root)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(asked);
    let _ = crate::ui::fixtures::rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("data-page=\"Spaces\""), "{page}");
    assert!(
        page.contains("value=\"Work\""),
        "not on the Space's page: {page}"
    );
}

/// Three Spaces, Home, Work and Travel in that order, written where the window reads them.
fn three(built: &Work) {
    let spaces = crate::ui::space::built(
        ["Home", "Work", "Travel"]
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    ds::prelude::SpaceLook::default(),
                    crate::ui::space::Mail::default(),
                )
            })
            .collect(),
        0,
    );
    crate::ui::space::save(Some(&built.dirs), &spaces).expect("the Spaces were written");
}

fn names(built: &Work) -> Vec<String> {
    on_disk(built)
        .list()
        .iter()
        .map(|one| one.name.clone())
        .collect()
}

/// Right-click the row of the Space named `name`, and wait for its menu.
async fn menu_of(dom: &mut VirtualDom, seen: &Seen, built: &Work, name: &str) -> Seen {
    let id = on_disk(built)
        .list()
        .iter()
        .find(|one| one.name == name)
        .expect("the Space")
        .id;
    let row = seen.one("id", &format!("spaces-pane-open-{}", id.0));
    let opened = right_click(dom, row);
    let mut more = settle_quiet_seen(dom).await;
    more = more.merge(opened);
    more
}

#[tokio::test]
async fn a_space_moves_up_and_down_the_order_from_its_row_s_right_click() {
    let built = work();
    three(&built);
    let (mut dom, seen) = opened_on(&built, SettingsPage::Spaces);

    let opened = menu_of(&mut dom, &seen, &built, "Home").await;
    assert_eq!(
        menu_names(&dioxus_ssr::render(&dom)),
        [copy::menus::MOVE_DOWN],
        "the first Space cannot move up"
    );
    pick_named(&mut dom, &opened, copy::menus::MOVE_DOWN).await;
    assert_eq!(names(&built), ["Work", "Home", "Travel"]);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.find(">Work<") < page.find(">Home<"),
        "the list is not in the new order: {page}"
    );

    let seen = settle_quiet_seen(&mut dom).await.merge(seen);
    let opened = menu_of(&mut dom, &seen, &built, "Travel").await;
    assert_eq!(
        menu_names(&dioxus_ssr::render(&dom)),
        [copy::menus::MOVE_UP],
        "the last Space cannot move down"
    );
    pick_named(&mut dom, &opened, copy::menus::MOVE_UP).await;
    assert_eq!(names(&built), ["Work", "Travel", "Home"]);
    assert_eq!(
        on_disk(&built).current().name,
        "Home",
        "the Space on screen changed"
    );
}

#[tokio::test]
async fn a_lone_space_s_right_click_opens_nothing() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::Spaces);
    let name = names(&built).remove(0);
    menu_of(&mut dom, &seen, &built, &name).await;
    assert!(
        menu_names(&dioxus_ssr::render(&dom)).is_empty(),
        "a menu with nowhere to move"
    );
}
