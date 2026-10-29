//! Settings › Spaces over the reference fixture: the list of Spaces, a Space's page pushed from
//! its row, and each of the page's edits written to the Spaces file the other windows read. Dragging
//! a Space into a new place is a lane (`tests/app/lanes/spaces.rs`): it takes a real pointer.

use super::super::tests::opened_on;
use crate::ui::copy;
use crate::ui::fixtures::{Work, click, drain_seen, type_into, work};
use crate::ui::space::{Spaces, stored};
use crate::ui::view::PageId;
use dioxus::dioxus_core::VirtualDom;
use ds::style::appearance::theme::Theme;
use ds::style::space::presets::PRESETS;

/// The Spaces as the file holds them now.
fn on_disk(built: &Work) -> Spaces {
    stored(&built.dirs).expect("the Spaces were written")
}

/// The Settings window on the Spaces page, with the first Space's page pushed.
fn first_space(built: &Work) -> (VirtualDom, crate::ui::fixtures::Seen) {
    let (mut dom, seen) = opened_on(built, PageId::SPACES);
    let page = dioxus_ssr::render(&dom);
    let at = page.find("spaces-pane-open-").expect("a Space's row");
    let id: String = page[at..].chars().take_while(|c| *c != '"').collect();
    let seen = click(&mut dom, seen.one("id", &id)).merge(drain_seen(&mut dom));
    (dom, seen)
}

#[test]
fn the_list_names_each_space_and_offers_a_new_one() {
    let built = work();
    let (dom, _) = opened_on(&built, PageId::SPACES);
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

/// The number of Spaces the list draws a row for.
fn rows(dom: &VirtualDom) -> usize {
    dioxus_ssr::render(dom).matches("spaces-pane-open-").count()
}

/// New Space opens a page for a Space that is not there yet: nothing is added until Create
/// Space, and then it is, named and tinted as drafted, and the pane goes back to the list.
#[test]
fn a_new_space_is_added_only_by_create_space() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::SPACES);
    let before = rows(&dom);
    let seen = click(&mut dom, seen.one("aria-label", copy::controls::NEW_SPACE))
        .merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    let name = format!("Space {}", before + 1);
    assert!(
        page.contains(&format!("value=\"{name}\"")),
        "its page: {page}"
    );
    assert_eq!(
        stored(&built.dirs).map_or(before, |spaces| spaces.count()),
        before,
        "a Space was added before Create Space"
    );
    let seen = type_into(
        &mut dom,
        seen.one("aria-label", copy::fields::SPACE_NAME),
        "Travel",
    )
    .merge(seen);
    click(
        &mut dom,
        seen.one("aria-label", copy::controls::CREATE_SPACE),
    );
    let after = on_disk(&built);
    assert_eq!(after.count(), before + 1, "Create Space added nothing");
    assert_eq!(
        after.list().last().map(|space| space.name.as_str()),
        Some("Travel")
    );
    assert_eq!(
        rows(&dom),
        before + 1,
        "the pane did not go back to the list"
    );
}

/// A new Space's page left with back, or with its name blank, adds nothing.
#[test]
fn a_new_space_left_or_unnamed_is_not_added() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::SPACES);
    let before = rows(&dom);
    let seen = click(&mut dom, seen.one("aria-label", copy::controls::NEW_SPACE))
        .merge(drain_seen(&mut dom));
    let seen = type_into(
        &mut dom,
        seen.one("aria-label", copy::fields::SPACE_NAME),
        " ",
    )
    .merge(seen);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(copy::errors::SPACE_NEEDS_A_NAME),
        "no error: {page}"
    );
    click(
        &mut dom,
        seen.one("aria-label", copy::controls::CREATE_SPACE),
    );
    let count = || stored(&built.dirs).map_or(before, |spaces| spaces.count());
    assert_eq!(count(), before, "a Space with no name was added");
    let back = seen.one("id", "spaces-pane-back-detail");
    click(&mut dom, back);
    assert_eq!(rows(&dom), before, "back added the Space");
    assert_eq!(count(), before, "back added the Space");
}

/// A Space made with Create Space can be deleted from its page, after asking.
#[test]
fn a_new_space_can_be_deleted_after_asking() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::SPACES);
    let before = rows(&dom);
    let seen = click(&mut dom, seen.one("aria-label", copy::controls::NEW_SPACE))
        .merge(drain_seen(&mut dom));
    let seen = click(
        &mut dom,
        seen.one("aria-label", copy::controls::CREATE_SPACE),
    )
    .merge(drain_seen(&mut dom))
    .merge(seen);
    let name = format!("Space {}", before + 1);
    let seen = click(
        &mut dom,
        seen.one("aria-label", &copy::aria::details_for(&name)),
    )
    .merge(drain_seen(&mut dom))
    .merge(seen);
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
        .with_root_context(crate::ui::fixtures::client(&built.store))
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
