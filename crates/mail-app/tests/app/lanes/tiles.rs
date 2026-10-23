//! Lane 12: an account tile's menu. A right click on the account's tile at the top of the sidebar
//! opens its menu: Sync Now, Remove from This Space (dimmed: the account is in no other Space),
//! and Account Settings…, and not the Space's menu the sidebar around it opens. Escape closes it,
//! and nothing changed.

use super::hands::{ms, until};
use super::look::menu_item_of;
use super::window::Window;
use crate::drive::Drive as _;
use ds::prelude::ShortcutKey as Key;

/// The seeded account's tile (`seed::ME`).
const TILE: &str = "[*|role=\"group\"][*|aria-label=\"Accounts in This Space\"] [*|aria-label=\"me@example.test\"]";
const MENU: &str = ".ds-menu[*|aria-label=\"Account\"]";

#[test]
fn a_right_click_on_an_account_tile_opens_its_menu() {
    let mut window = Window::open(|_| {});
    let h = &mut window.harness;
    until(h, "the account's tile is drawn", |h| h.count(TILE) == 1);
    let at = super::hands::centre(h, TILE);
    h.press(at, ds::base::press::PointerButton::Secondary);
    until(h, "the tile's menu opens", |h| h.count(MENU) == 1);
    // The tile took the click: the sidebar under it does not open the Space's menu as well.
    assert_eq!(h.count(".ds-menu"), 1, "the Space's menu opened too");

    let names: Vec<String> = (1..=h.count(".ds-menu-list > *"))
        .filter_map(|n| h.text_of(&format!(".ds-menu-list > :nth-child({n}) .ds-menu-label")))
        .map(|label| label.trim().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "Sync Now",
            "Remove from This Space",
            "Account Settings\u{2026}"
        ],
        "{names:?}"
    );
    let remove = menu_item_of(h, "Remove from This Space").unwrap_or_default();
    assert!(
        h.attr(
            &format!(".ds-menu-list > :nth-child({remove})"),
            "aria-disabled"
        )
        .is_none_or(|disabled| disabled != "true"),
        "an account in no other Space cannot be taken out of this one: it may be in none"
    );

    h.key(Key::Escape);
    h.advance(ms(300));
    assert_eq!(h.count(MENU), 0, "Escape left the menu up");
    assert_eq!(h.count(TILE), 1, "the tile went");
}
