mod folder_act;
mod folder_parts;
mod folder_row;
mod folder_tree;
mod folders;
mod join;
mod marks;
mod panes;
mod today;

pub(super) use self::folder_act::folder_places;
use self::folder_tree::{Show, arrange, scope};
use self::folders::FolderList;
pub(in crate::ui) use self::panes::hex_colour;
use self::panes::{AccountTiles, PinnedList, PlaceList, counts};
use self::today::TodayList;
use crate::ui::appearance::WindowDirs;
use crate::ui::space::{Handle, Mail, Scope};
use crate::ui::today::Today;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::edge_peek::EdgePeek;
use ds::components::app::spaces::{SpaceHead, SpacesFoot};
use ds::components::controls::button_model::ImagePosition;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName, ExtraClass};

/// A `data-<name>` of mailo's own on a quire component: where a hover card or a test finds the
/// thing the element stands for.
pub(super) fn tagged(name: &str, value: impl Into<String>) -> Common {
    Common {
        data: DataName::parse(name)
            .map(|name| vec![DataAttr::new(name, value)])
            .unwrap_or_default(),
        ..Common::default()
    }
}

/// `common`, with the class that quietens a folder the person does not follow.
pub(super) fn dimmed(common: Common) -> Common {
    Common {
        extra_class: ExtraClass::parse("dim").ok(),
        ..common
    }
}

/// The coloured sidebar.
#[component]
pub(super) fn Places(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    revision: Signal<u64>,
    handle: Handle,
    today: Signal<Today>,
    dirs: Option<WindowDirs>,
    side_hidden: Signal<bool>,
) -> Element {
    let spaces = handle.spaces();
    let space = spaces.read().current().clone();
    let counted = use_memo(move || {
        let _ = revision();
        let store = consume_context::<std::sync::Arc<mail_core::SqliteStore>>();
        counts(&store, spaces.read().current())
    });
    // The accounts the Folders section is for. A memo, so a keystroke in the search box — a
    // shell change — does not read the folders again.
    let in_scope = use_memo(move || scope(&shell.read()));
    let show = use_signal(|| Show::Followed);
    let folders = use_memo(move || {
        let _ = revision();
        let store = consume_context::<std::sync::Arc<mail_core::SqliteStore>>();
        arrange(&folder_act::load(&store, &in_scope()), show())
    });
    let folded = folders
        .read()
        .as_ref()
        .map(|section| section.labels.clone())
        .unwrap_or_default();
    let space_id = space.id;
    let tiles = counted.read().clone();
    let pinned = if side_hidden() {
        Shown::Hidden
    } else {
        Shown::Visible
    };
    rsx! {
        // Hidden, the sidebar floats over the card from the edge while the pointer is there
        // (quire's `EdgePeek`), and a click on the edge pins it again.
        EdgePeek {
            label: "Sidebar",
            pinned,
            onpin: move |()| side_hidden.set(false),
            common: Common { extra_class: ExtraClass::parse("side").ok(), ..Common::default() },
            // The Space's name heads the sidebar, as a browser's does; a right click on it is the
            // Space's menu, and a plain press does nothing.
            SpaceHead { handle }
            div { class: "slide",
                AccountTiles { shell, pages, spaces, space: space.clone(), counted: tiles.clone() }
                PlaceList { shell, pages, badges, folded }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, revision, section, show }
                }
                PinnedList { shell, pages, space: space.clone(), pins: tiles.pins.clone() }
                TodayList { shell, today, space: space_id, dirs: dirs.clone() }
            }
            // Downloads, the Space dots, New Space, then Settings and the sidebar's toggle.
            SpacesFoot {
                handle,
                new_payload: move |()| Mail::over(Scope::All),
                leading: rsx! { crate::ui::downloads::DownloadsButton { shell } },
                trailing: rsx! {
                    Button {
                        bezel: ds::components::controls::button_model::Bezel::Toolbar,
                        image: ImagePosition::Only,
                        icon: Icon::Settings,
                        label: "Settings",
                        title: "Settings".to_owned(),
                        title_shortcut: crate::ui::actions::tip_standard(chordkit::StandardAction::Settings),
                        onclick: move |_| crate::ui::settings_window::open(),
                    }
                    Button {
                        bezel: ds::components::controls::button_model::Bezel::Toolbar,
                        image: ImagePosition::Only,
                        icon: Icon::PanelLeft,
                        label: if side_hidden() { "Show sidebar" } else { "Hide sidebar" },
                        title: "Sidebar".to_owned(),
                        title_shortcut: crate::ui::actions::tip_standard(chordkit::StandardAction::ToggleSidebar),
                        onclick: move |_| side_hidden.set(!side_hidden()),
                    }
                },
            }
        }
    }
}
#[cfg(test)]
mod folder_place_tests;
#[cfg(test)]
mod folder_store_tests;
#[cfg(test)]
mod folder_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;
