//! The Space's menu over the reference fixture: a right click on the Space's name opens it with
//! every part of the Space; Rename, Colour and Delete open their popovers where it stood; a
//! rename shows at once and is kept when the popover closes, by Escape as by Return; Delete asks,
//! and only its button deletes.

use crate::ui::app::App;
use crate::ui::fixtures::{Seen, Work, chord, dispatching, later, rebuild_into, right_click, work};
use crate::ui::space;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};

/// The renders after an event: quire's menu and popovers float in the root's overlay, drawn a
/// render or two after they are asked for.
fn settle(dom: &mut VirtualDom, mut seen: Seen) -> Seen {
    for _ in 0..8 {
        dom.process_events();
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    seen
}

fn click(dom: &mut VirtualDom, element: ElementId) -> Seen {
    let seen = crate::ui::fixtures::click(dom, element);
    settle(dom, seen)
}

/// Pick a row of the menu: quire blinks the row, closes the menu, then acts, on its clock.
async fn pick(dom: &mut VirtualDom, element: ElementId) -> Seen {
    let seen = click(dom, element);
    seen.merge(later(dom).await)
}

/// The window on the Work Space, with a second Space beside it so Delete is offered, and Work's
/// menu open from a right click on its name. The `Work` comes back too: it owns the directories
/// the window writes into.
fn menu_open() -> (VirtualDom, Seen, Work) {
    dispatching();
    let built = work();
    let mut spaces = space::load(&built.dirs.config);
    let mut home = spaces.current_space();
    home.name = "Home".to_owned();
    spaces.spaces.push(home);
    space::save(&built.dirs.config, &spaces).expect("the fixture's Spaces are written");
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let opened = right_click(&mut dom, seen.one("aria-label", "The Work Space"));
    let seen = settle(&mut dom, seen.merge(opened));
    (dom, seen, built)
}

/// The menu's rows in order: Rename, Colour, Appearance, Accent, Accounts, New Space, Delete.
fn row(seen: &Seen, n: usize) -> ElementId {
    seen.fixed("class", "ds-menu-item")[n]
}

const RENAME: usize = 0;
const COLOUR: usize = 1;
const DELETE: usize = 6;

/// The frame with each of the Space's parts open in turn, for the stylesheet's class check.
pub(in crate::ui) async fn parts_open_markup() -> String {
    let mut out = String::new();
    for part in [RENAME, COLOUR, DELETE] {
        let (mut dom, seen, _built) = menu_open();
        let _ = pick(&mut dom, row(&seen, part)).await;
        out.push_str(&dioxus_ssr::render(&dom));
    }
    out
}

#[tokio::test]
async fn a_right_click_on_the_space_lists_its_parts() {
    let (dom, _seen, _built) = menu_open();
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("aria-label=\"Work Space\""), "{page}");
    for item in [
        "Rename\u{2026}",
        "Colour\u{2026}",
        "Appearance",
        "Accent Inside the Card",
        "Accounts",
        "New Space",
        "Delete Space\u{2026}",
    ] {
        assert!(page.contains(item), "{item} missing: {page}");
    }
    assert!(
        !page.contains("aria-label=\"Space editor\""),
        "the old editor sheet opened: {page}"
    );
}

#[tokio::test]
async fn a_rename_shows_at_once_and_escape_keeps_it() {
    let (mut dom, seen, built) = menu_open();
    let picked = pick(&mut dom, row(&seen, RENAME)).await;
    let seen = seen.merge(picked);
    let field = seen.one("aria-label", "Space name");
    let _ = crate::ui::fixtures::type_into(&mut dom, field, "Studio");
    let live = dioxus_ssr::render(&dom);
    assert!(
        live.contains("aria-label=\"The Studio Space\""),
        "the name did not reach the sidebar live: {live}"
    );
    // Escape on the popover, which closes the topmost layer of quire's stack.
    let popover = seen.one("aria-label", "Rename Space");
    let _ = chord(&mut dom, "Escape", Modifiers::empty(), popover);
    let _ = later(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("aria-label=\"Rename Space\""),
        "Escape left the popover open: {page}"
    );
    assert_eq!(
        space::load(&built.dirs.config).current_space().name,
        "Studio"
    );
}

#[tokio::test]
async fn return_closes_the_rename_and_keeps_it() {
    let (mut dom, seen, built) = menu_open();
    let picked = pick(&mut dom, row(&seen, RENAME)).await;
    let seen = seen.merge(picked);
    let field = seen.one("aria-label", "Space name");
    let _ = crate::ui::fixtures::type_into(&mut dom, field, "Desk");
    let _ = chord(&mut dom, "Enter", Modifiers::empty(), field);
    let _ = settle(&mut dom, Seen::default());
    assert!(
        !dioxus_ssr::render(&dom).contains("aria-label=\"Rename Space\""),
        "Return left the popover open"
    );
    assert_eq!(space::load(&built.dirs.config).current_space().name, "Desk");
}

#[tokio::test]
async fn colour_opens_the_colour_half_of_quire_s_editor() {
    let (mut dom, seen, _built) = menu_open();
    let _ = pick(&mut dom, row(&seen, COLOUR)).await;
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("data-part=\"colour\""), "{page}");
    assert!(
        !page.contains("aria-label=\"Appearance\""),
        "the colour popover drew the whole editor: {page}"
    );
}

#[tokio::test]
async fn delete_asks_and_only_its_button_deletes() {
    let (mut dom, seen, built) = menu_open();
    let before = space::load(&built.dirs.config).spaces.len();
    assert!(before > 1, "the fixture has one Space");
    let picked = pick(&mut dom, row(&seen, DELETE)).await;
    let seen = seen.merge(picked);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Delete \u{201c}Work\u{201d}?"), "{page}");
    assert_eq!(space::load(&built.dirs.config).spaces.len(), before);
    let _ = click(&mut dom, seen.one("aria-label", "Delete Space"));
    assert_eq!(space::load(&built.dirs.config).spaces.len(), before - 1);
    assert!(
        !dioxus_ssr::render(&dom).contains("Delete \u{201c}Work\u{201d}?"),
        "the question stayed"
    );
}

#[tokio::test]
async fn the_parts_lint_clean() {
    let offences = crate::ui::style::tests::markup_offences(&parts_open_markup().await);
    assert!(offences.is_empty(), "the markup lint: {offences:#?}");
}
