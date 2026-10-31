//! The sidebar's one menu, under the chevron at the right of the foot: Tidy Up…, Settings and
//! the sidebar's own toggle.
//!
//! Everything else the foot once listed has a home of its own: what was opened lately and the
//! drafts put aside are ⌘K's Recent, History is in ⌘K, and New Space is in the Space's menu, a
//! right click on the foot. What the rows are is data ([`menu_items`]) and pure, so a table test
//! can say what the menu lists; [`MoreMenu`] draws the button and acts on a pick.

use crate::ui::actions::tip_standard;
use crate::ui::command::{Acts, Command, Settings, Sidebar, TidyUp, perform};
use dioxus::prelude::*;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_ui_model::copy;

/// The menu's rows: Tidy Up…, Settings and the sidebar's toggle, named by whether the sidebar is pinned
/// open (`sidebar`).
pub(super) fn menu_items(sidebar: Shown) -> Vec<MenuItem<&'static dyn Command>> {
    let toggle = match sidebar {
        Shown::Visible => copy::menus::HIDE_SIDEBAR,
        Shown::Hidden => copy::menus::SHOW_SIDEBAR,
    };
    vec![
        row(&TidyUp, TidyUp.label()),
        MenuItem::Separator,
        row(&Settings, Settings.label()).with_key(tip_standard(chordkit::StandardAction::Settings)),
        row(&Sidebar, toggle).with_key(tip_standard(chordkit::StandardAction::ToggleSidebar)),
    ]
}

/// A row for `command`, drawn with the icon ⌘K gives it: the foot lists the same commands, so
/// they look alike in both. `name` is the row's words, which the sidebar's toggle changes with
/// its state.
fn row(command: &'static dyn Command, name: &str) -> MenuItem<&'static dyn Command> {
    MenuItem::new(command, name).with_image(MenuImage::Icon(command.icon()))
}

/// The chevron at the foot's right and the menu it opens: quire's pull-down in its chevron face.
#[component]
pub(in crate::ui) fn MoreMenu(
    shell: Signal<crate::ui::view::Shell>,
    pages: Signal<u32>,
    side_hidden: Signal<bool>,
) -> Element {
    let mut open = use_signal(|| Shown::Hidden);
    let mut tool = use_signal(|| None::<MountedRef>);
    let items = if open() == Shown::Visible {
        menu_items(if side_hidden() {
            Shown::Hidden
        } else {
            Shown::Visible
        })
    } else {
        Vec::new()
    };
    rsx! {
        PullDownButton::<&'static dyn Command> {
            face: PullDownFace::Chevron,
            items,
            open: Some(open()),
            on_open_change: move |to: Shown| open.set(to),
            // A window with no renderer (the tests) has no rect to hang the menu from.
            anchor: Some(crate::ui::menu::anchor_at(tool())),
            common: Common {
                aria_label: Some(copy::aria::SIDEBAR_MENU.to_owned()),
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    tool.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onpick: move |command: &'static dyn Command| {
                perform(
                    command,
                    Acts {
                        shell,
                        pages,
                        side_hidden,
                    },
                );
            },
        }
    }
}

#[cfg(test)]
mod tests;
