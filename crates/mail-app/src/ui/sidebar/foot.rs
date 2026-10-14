//! The sidebar's one menu, under the chevron at the right of the foot: Tidy Up…, Settings and
//! the sidebar's own toggle.
//!
//! Everything else the foot once listed has a home of its own: what was opened lately and the
//! drafts put aside are ⌘K's Recent, History is in ⌘K, and New Space is in the Space's menu, a
//! right click on the foot. What the rows are is data ([`menu_items`]) and pure, so a table test
//! can say what the menu lists; [`MoreMenu`] draws the button and acts on a pick.

use crate::ui::actions::tip_standard;
use crate::ui::copy;
use dioxus::prelude::*;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;

/// What a row of the menu asks for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Pick {
    /// Open Tidy Up, which archives the Inbox's old read conversations.
    TidyUp,
    /// Open the settings window.
    Settings,
    /// Hide the sidebar, or show it again.
    ToggleSidebar,
}

/// The menu's rows: Tidy Up…, Settings and the sidebar's toggle, named by whether the sidebar is pinned
/// open (`sidebar`).
pub(super) fn menu_items(sidebar: Shown) -> Vec<MenuItem<Pick>> {
    let toggle = match sidebar {
        Shown::Visible => copy::menus::HIDE_SIDEBAR,
        Shown::Hidden => copy::menus::SHOW_SIDEBAR,
    };
    vec![
        MenuItem::new(Pick::TidyUp, copy::menus::TIDY_UP)
            .with_image(MenuImage::Icon(Icon::Archive)),
        MenuItem::Separator,
        MenuItem::new(Pick::Settings, copy::menus::SETTINGS)
            .with_image(MenuImage::Icon(Icon::Settings))
            .with_key(tip_standard(chordkit::StandardAction::Settings)),
        MenuItem::new(Pick::ToggleSidebar, toggle)
            .with_image(MenuImage::Icon(Icon::PanelLeft))
            .with_key(tip_standard(chordkit::StandardAction::ToggleSidebar)),
    ]
}

/// The chevron at the foot's right and the menu it opens: quire's pull-down in its chevron face.
#[component]
pub(in crate::ui) fn MoreMenu(
    shell: Signal<crate::ui::view::Shell>,
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
        PullDownButton::<Pick> {
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
            onpick: move |pick: Pick| match pick {
                Pick::TidyUp => crate::ui::tidy::open(shell),
                Pick::Settings => crate::ui::settings_window::open(),
                Pick::ToggleSidebar => side_hidden.set(!side_hidden()),
            },
        }
    }
}

#[cfg(test)]
mod tests;
