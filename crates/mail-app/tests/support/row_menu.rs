//! A conversation row's menu, driven the way a person does: a right click on the row, then the
//! item that says what to do. A row has no action buttons of its own; its one button is the ⋯,
//! and every action is in this menu.
//!
//! Included by the integration tests through `#[path]`, beside `settle.rs` and `drive.rs`.

#![allow(dead_code)]

use super::drive::Drive;
use super::settle::settle_until;
use ds::base::press::PointerButton;
use ds_harness::{Harness, Query};

/// The open menu's children, items and rules alike, by position (1-based).
fn child(n: usize) -> String {
    format!(".ds-menu > :nth-child({n})")
}

/// Right-click the row `row` (a selector for its list item) on its subject line, and wait for
/// its menu to open.
pub fn open_row_menu(harness: &mut Harness, row: &str) {
    let line = format!("{row} .ds-thread-sub");
    let at = harness
        .centre(&line)
        .unwrap_or_else(|| panic!("{line} is not drawn:\n{}", harness.html()));
    harness.press(at, PointerButton::Secondary);
    settle_until(harness, |h| h.count(".ds-menu .ds-menu-item") > 0);
}

/// The open menu's item names, in order.
pub fn menu_names(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".ds-menu > *"))
        .filter_map(|n| harness.text_of(&format!("{} .ds-menu-label", child(n))))
        .collect()
}

/// Press the open menu's item named `name` once it stands where it is drawn, and wait for the
/// menu to go: quire blinks the item, then acts. A pick that opens a further menu (Snooze…,
/// Label…, Move to…) leaves that one up.
pub fn press_menu_item(harness: &mut Harness, name: &str) {
    let n = (1..=harness.count(".ds-menu > *"))
        .find(|n| {
            harness
                .text_of(&format!("{} .ds-menu-label", child(*n)))
                .is_some_and(|label| label.trim() == name)
        })
        .unwrap_or_else(|| {
            panic!(
                "no menu item is named {name:?}: {:?}\n{}",
                menu_names(harness),
                harness.html()
            )
        });
    let item = child(n);
    settle_until(harness, |h| {
        h.centre(&item).is_some_and(|at| h.hits(at, &item))
    });
    let at = harness.centre(&item).unwrap_or_default();
    harness.click(at);
    let label = format!("{item} .ds-menu-label");
    settle_until(harness, |h| {
        !h.text_of(&label).is_some_and(|text| text.trim() == name)
    });
}

/// Right-click the row `row` and pick `name` from its menu.
pub fn row_action(harness: &mut Harness, row: &str, name: &str) {
    open_row_menu(harness, row);
    press_menu_item(harness, name);
}
