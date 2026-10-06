//! A Space's menu: a right click on the Space (its dot, its name, the sidebar's foot) or a click
//! on its name opens quire's context `Menu` at the pointer, and each part of the Space is one of
//! its rows (`pick.rs`). Appearance, the card's accent and the accounts it shows are submenus of
//! checks that change the Space at once; Rename, Colour and Delete each open a small popover
//! where the menu stood (`popovers.rs`, `remove.rs`). There is no editor sheet: a Space is tuned
//! a part at a time, near the pointer, and everything that applies to every Space is in Settings.
//!
//! A part's popover changes the Space live, as the frame repaints with each change, and keeps
//! what it shows when it closes, by Escape or a click outside alike: a popover has no Cancel.
//! While one is open its Space is held as a [`Draft`] in `editing`, so the window's other effects
//! (an account removed, a Settings window writing `spaces.json`) wait for it to close.

mod parts;
mod pick;
mod popovers;
mod remove;

pub(in crate::ui) use parts::Seg;

use self::pick::{Pick, Then, apply, rows};
use super::frame::keep;
use crate::ui::data::account_rows;
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::today::Today;
use crate::ui::view::{Shell, SpaceMenu, SpaceShowing};
use dioxus::prelude::*;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// A point in the window's client pixels as the menu keeps it.
fn pixel(at: (f64, f64)) -> (i32, i32) {
    (at.0.round() as i32, at.1.round() as i32)
}

/// Open Space `index`'s menu at `at`, a point in the window's client pixels.
pub(in crate::ui) fn open(mut shell: Signal<Shell>, index: usize, at: (f64, f64)) {
    shell.write().space_menu = Some(SpaceMenu {
        index,
        at: pixel(at),
        showing: SpaceShowing::Menu,
    });
}

/// Open one of Space `index`'s parts at `at` without the menu first ("+" opens the new Space's
/// name), holding the Space as a draft while it shows.
pub(in crate::ui) fn open_part(
    mut shell: Signal<Shell>,
    spaces: Signal<Spaces>,
    mut editing: Signal<Option<Draft>>,
    index: usize,
    at: (f64, f64),
    showing: SpaceShowing,
) {
    let held = spaces.read().spaces.get(index).cloned();
    if let Some(space) = held {
        editing.set(Some(Draft::open(index, space)));
    }
    shell.write().space_menu = Some(SpaceMenu {
        index,
        at: pixel(at),
        showing,
    });
    // The keyboard goes where Escape can close the part: the Delete question's Cancel, the safe
    // answer, as an alert's default; the window for the colour, whose Escape `App` takes. The
    // name field asks for the keyboard itself.
    match showing {
        SpaceShowing::Delete => crate::ui::host::Host::focus_next_frame(".space-delete button"),
        SpaceShowing::Colour => crate::ui::host::Host::focus_app(),
        SpaceShowing::Menu | SpaceShowing::Rename => {}
    }
}

/// Close the menu or the part showing, keeping what a part changed, and give the keyboard back.
pub(in crate::ui) fn close(
    mut shell: Signal<Shell>,
    mut editing: Signal<Option<Draft>>,
    spaces: Signal<Spaces>,
) {
    if editing.peek().is_some() {
        editing.set(None);
        keep(&spaces.read());
    }
    if shell.peek().space_menu.is_some() {
        shell.write().space_menu = None;
    }
    crate::ui::host::Host::focus_app();
}

/// Apply `edit` to the draft a part holds and put the result in the window's Spaces, which the
/// frame's root reads. Nothing is written until the part closes.
pub(super) fn change(
    mut editing: Signal<Option<Draft>>,
    mut spaces: Signal<Spaces>,
    edit: impl FnOnce(&mut Draft),
) {
    let (index, space) = {
        let mut guard = editing.write();
        let Some(draft) = guard.as_mut() else {
            return;
        };
        edit(draft);
        (draft.index, draft.space.clone())
    };
    if let Some(slot) = spaces.write().spaces.get_mut(index) {
        *slot = space;
    }
}

/// Every account there is, in the window's order, with how it is named.
fn accounts() -> Vec<(AccountId, String)> {
    try_consume_context::<Arc<SqliteStore>>()
        .map(|store| {
            account_rows(&store)
                .into_iter()
                .map(|row| (row.id.clone(), row.shown()))
                .collect()
        })
        .unwrap_or_default()
}

/// A pick from Space `index`'s menu, opened at `at`.
fn picked(
    pick: Pick,
    index: usize,
    at: (f64, f64),
    mut spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
    pages: Signal<u32>,
) {
    let all: Vec<AccountId> = accounts().into_iter().map(|(id, _)| id).collect();
    let Some(mut space) = spaces.read().spaces.get(index).cloned() else {
        return;
    };
    match apply(pick, &mut space, &all) {
        Then::Kept => {
            if let Some(slot) = spaces.write().spaces.get_mut(index) {
                *slot = space;
            }
            keep(&spaces.read());
        }
        Then::Open(showing) => open_part(shell, spaces, editing, index, at, showing),
        Then::NewSpace => {
            let made = crate::ui::switch::add(spaces, shell, pages);
            open_part(shell, spaces, editing, made, at, SpaceShowing::Rename);
        }
    }
}

/// The menu, or the part it opened. Renders nothing while `shell.space_menu` is `None`.
#[component]
pub(super) fn SpaceMenuView(
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    today: Signal<Today>,
) -> Element {
    let Some(menu) = shell.read().space_menu else {
        return rsx! {};
    };
    let Some(space) = spaces.read().spaces.get(menu.index).cloned() else {
        return rsx! {};
    };
    let at = Point {
        x: Px(menu.at.0 as f32),
        y: Px(menu.at.1 as f32),
    };
    let point = (f64::from(menu.at.0), f64::from(menu.at.1));
    let index = menu.index;
    match menu.showing {
        SpaceShowing::Menu => rsx! {
            Menu::<Pick> {
                placement: MenuPlacement::Context,
                anchor: Anchor::Point(at),
                items: rows(&space, spaces.read().spaces.len(), &accounts()),
                common: Common {
                    aria_label: Some(format!("{} Space", space.name)),
                    ..Common::default()
                },
                onpick: move |pick: Pick| picked(pick, index, point, spaces, editing, shell, pages),
                // A pick that opened a part has already put the part in the menu's place: only
                // the menu itself closes here.
                onclose: move |()| {
                    let showing = shell.peek().space_menu.map(|open| open.showing);
                    if showing == Some(SpaceShowing::Menu) {
                        close(shell, editing, spaces);
                    }
                },
            }
        },
        SpaceShowing::Rename => rsx! {
            popovers::RenamePopover { at, spaces, editing, shell }
        },
        SpaceShowing::Colour => rsx! {
            popovers::ColourPopover { at, spaces, editing, shell }
        },
        SpaceShowing::Delete => rsx! {
            remove::DeletePopover { at, index, spaces, editing, shell, pages, today }
        },
    }
}

#[cfg(test)]
pub(in crate::ui) mod tests;
