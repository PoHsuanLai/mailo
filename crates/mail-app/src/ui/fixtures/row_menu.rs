//! Driving a conversation row's menu, the one place a row's actions are: a right click on the
//! row opens it, and an item is picked by the name it shows.

use super::dom::{Seen, click, right_click};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;

/// The renders after an event: quire's menu floats in the root's overlay, drawn a render or two
/// after it is asked for.
fn settle(dom: &mut VirtualDom, mut seen: Seen) -> Seen {
    for _ in 0..8 {
        dom.process_events();
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    seen
}

/// Right-click the row whose subject is `subject`, found in `seen`, and return what the renders
/// that drew its menu set.
pub(in crate::ui) fn open_row_menu(dom: &mut VirtualDom, seen: &Seen, subject: &str) -> Seen {
    let row = row_named(seen, subject);
    let opened = right_click(dom, row);
    settle(dom, opened)
}

/// The row whose subject is `subject`, as the last render that drew it gave it out.
pub(in crate::ui) fn row_named(seen: &Seen, subject: &str) -> dioxus_core::ElementId {
    *seen
        .all("aria-label", &format!("Open {subject}"))
        .last()
        .unwrap_or_else(|| panic!("no row is about {subject:?}"))
}

/// The subjects of the rows the document lists, in order.
pub(in crate::ui) fn listed_subjects(page: &str) -> Vec<String> {
    page.split("aria-label=\"Open ")
        .skip(1)
        .filter_map(|rest| Some(rest.split('"').next()?.to_owned()))
        .collect()
}

/// The names of the open menu's items, in order, as the document draws them.
pub(in crate::ui) fn menu_names(page: &str) -> Vec<String> {
    page.split("class=\"ds-menu-item\"")
        .skip(1)
        .filter_map(|item| {
            let label = item.split("class=\"ds-menu-label\">").nth(1)?;
            Some(label.split('<').next()?.to_owned())
        })
        .collect()
}

/// Pick the open menu's item named `name`, `opened` being what the renders that drew the menu
/// set. Quire blinks the item, closes the menu, then acts, on its clock; what the window drew
/// over that time comes back.
pub(in crate::ui) async fn pick_named(dom: &mut VirtualDom, opened: &Seen, name: &str) -> Seen {
    pick_until(dom, opened, name, || false).await
}

/// [`pick_named`], drawing only until `done` holds (or the blink's second has passed), for a test
/// that looks at what the pick started before it has finished: a row's exit, under way.
pub(in crate::ui) async fn pick_until(
    dom: &mut VirtualDom,
    opened: &Seen,
    name: &str,
    done: impl Fn() -> bool,
) -> Seen {
    let names = menu_names(&dioxus_ssr::render(dom));
    let index = names
        .iter()
        .position(|item| item == name)
        .unwrap_or_else(|| panic!("no menu item is named {name:?}: {names:?}"));
    let items = opened.fixed("class", "ds-menu-item");
    assert_eq!(
        items.len(),
        names.len(),
        "the menu drew {names:?}, but {} items were seen",
        items.len()
    );
    let clicked = click(dom, items[index]);
    let mut seen = settle(dom, clicked);
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
    while !done() && tokio::time::Instant::now() < until {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(16), dom.wait_for_work()).await;
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
        tokio::time::sleep(std::time::Duration::from_millis(4)).await;
    }
    seen
}

/// Right-click the row whose subject is `subject` and pick `name` from its menu.
pub(in crate::ui) async fn row_action(
    dom: &mut VirtualDom,
    seen: &Seen,
    subject: &str,
    name: &str,
) -> Seen {
    let opened = open_row_menu(dom, seen, subject);
    pick_named(dom, &opened, name).await
}
