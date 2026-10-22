//! A Space's place in the order, from its row's right click in Settings › Spaces: Move Up and
//! Move Down, where it can move (quire's lists have no drag to reorder, as Rules' order is moved
//! too). The order is the sidebar's and ⌘1 to ⌘9's; the Space on screen stays on screen, and the
//! order is written at once, for the other windows to read.

use crate::ui::copy;
use crate::ui::menu::{Floating, MenuItem};
use crate::ui::space::{Handle, SpaceId};
use dioxus::prelude::*;
use ds::prelude::*;

/// The keys of a Space's menu.
const UP: &str = "up";
const DOWN: &str = "down";

/// Which Space's menu is open, and where the right click was.
pub(super) type Open = Signal<Option<(SpaceId, Rect)>>;

/// The menu's rows for the Space at `index` of `count`: Move Up and Move Down only where it can
/// move.
pub(super) fn items(index: usize, count: usize) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if index > 0 {
        items.push(MenuItem::icon(UP, Icon::ChevronUp, copy::menus::MOVE_UP));
    }
    if index + 1 < count {
        items.push(MenuItem::icon(
            DOWN,
            Icon::ChevronDown,
            copy::menus::MOVE_DOWN,
        ));
    }
    items
}

/// The menu a right click on a Space's row opens, at the pointer.
#[component]
pub(super) fn SpaceMenu(handle: Handle, space: SpaceId, at: Rect, open: Open) -> Element {
    let (index, count, title) = {
        let held = handle.spaces();
        let spaces = held.read();
        let Some(index) = spaces.index_of(space) else {
            // Deleted from elsewhere while the menu was open.
            return rsx! {};
        };
        let title = spaces
            .get(space)
            .map(|one| one.name.clone())
            .unwrap_or_default();
        (index, spaces.count(), title)
    };
    let mut open = open;
    rsx! {
        Floating {
            anchor: None,
            placed: Some(at),
            title,
            items: items(index, count),
            on_pick: move |key: String| {
                open.set(None);
                match key.as_str() {
                    UP => move_to(handle, space, index.saturating_sub(1)),
                    DOWN => move_to(handle, space, index + 1),
                    _ => {}
                }
            },
            on_close: move |()| open.set(None),
        }
    }
}

/// Put `space` at `index` of the order, and write it.
fn move_to(handle: Handle, space: SpaceId, index: usize) {
    let mut spaces = handle.spaces();
    spaces.write().move_to(space, index);
    handle.commit();
}
