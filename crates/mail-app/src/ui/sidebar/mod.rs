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
use super::switch;
use crate::ui::appearance::WindowDirs;
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::today::Today;
use crate::ui::view::{Shell, SpaceShowing};
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::components::app::edge_peek::EdgePeek;
use ds::components::controls::button_model::ImagePosition;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName, ExtraClass};
use ds::style::space::frame_vars::FrameVars;

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
    spaces: Signal<Spaces>,
    today: Signal<Today>,
    dirs: Option<WindowDirs>,
    side_hidden: Signal<bool>,
    editing: Signal<Option<Draft>>,
) -> Element {
    let space = spaces.read().current_space();
    let counted = use_memo(move || {
        let _ = revision();
        let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
        counts(&store, &spaces.read().current_space())
    });
    // The accounts the Folders section is for. A memo, so a keystroke in the search box — a
    // shell change — does not read the folders again.
    let in_scope = use_memo(move || scope(&shell.read()));
    let show = use_signal(|| Show::Followed);
    let folders = use_memo(move || {
        let _ = revision();
        let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
        arrange(&folder_act::load(&store, &in_scope()), show())
    });
    let folded = folders
        .read()
        .as_ref()
        .map(|section| section.labels.clone())
        .unwrap_or_default();
    let space_index = spaces.read().current;
    let scheme = use_scope().scheme;
    let tiles = counted.read().clone();
    let name = space.name.clone();
    // The Space's menu at the pointer, from a right click anywhere on the foot.
    let open_menu = move |event: MouseEvent| {
        event.prevent_default();
        let at = event.client_coordinates();
        crate::ui::space_menu::open(shell, space_index, (at.x, at.y));
    };
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
            div { class: "slide",
                AccountTiles { shell, pages, spaces, space: space.clone(), counted: tiles.clone() }
                PlaceList { shell, pages, badges, folded }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, revision, section, show }
                }
                PinnedList { shell, pages, space: space.clone(), pins: tiles.pins.clone() }
                TodayList { shell, today, space_index, dirs: dirs.clone() }
            }
            div { class: "side-foot",
                oncontextmenu: open_menu,
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Inline,
                    label: name.clone(),
                    title: "This Space's menu".to_owned(),
                    common: Common {
                        aria_label: Some(format!("The {name} Space")),
                        extra_class: ExtraClass::parse("space-name").ok(),
                        ..Common::default()
                    },
                    onclick: move |press: Press| {
                        let at = (f64::from(press.at.x.0), f64::from(press.at.y.0));
                        crate::ui::space_menu::open(shell, space_index, at);
                    },
                }
                div { class: "space-dots", role: "group", aria_label: "Spaces",
                    for (index, one) in spaces.read().spaces.iter().enumerate() {
                        {
                            let selection = Selection::of(&index, &space_index);
                            // ⌘ and the Space's place, one to nine: the keys `App` switches on.
                            let keys = char::from_digit(u32::try_from(index + 1).unwrap_or(0), 10)
                                .map_or_else(Vec::new, |digit| vec![ShortcutKey::Super, ShortcutKey::Char(digit)]);
                            rsx! {
                                // A right click on a dot is that Space's menu, not the current one's.
                                span { key: "{index}", class: "space-dot-hold",
                                    oncontextmenu: move |event: MouseEvent| {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        let at = event.client_coordinates();
                                        crate::ui::space_menu::open(shell, index, (at.x, at.y));
                                    },
                                SpaceDot {
                                    name: one.name.clone(),
                                    frame: FrameVars::of(&one.look, scheme),
                                    selection,
                                    shortcut: Shortcut(keys),
                                    onclick: move |()| {
                                        if editing.read().is_none() {
                                            switch::go(spaces, shell, pages, index);
                                        }
                                    },
                                }
                                }
                            }
                        }
                    }
                }
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    icon: Icon::Plus,
                    label: "New Space",
                    title: "New Space".to_owned(),
                    onclick: move |press: Press| {
                        if editing.read().is_none() {
                            let made = switch::add(spaces, shell, pages);
                            let at = (f64::from(press.at.x.0), f64::from(press.at.y.0));
                            crate::ui::space_menu::open_part(
                                shell, spaces, editing, made, at, SpaceShowing::Rename,
                            );
                        }
                    },
                }
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    icon: Icon::Settings,
                    label: "Settings",
                    title: "Settings (\u{2318},)".to_owned(),
                    onclick: move |_| crate::ui::settings_window::open(),
                }
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    icon: Icon::PanelLeft,
                    label: if side_hidden() { "Show sidebar" } else { "Hide sidebar" },
                    title: if side_hidden() {
                        "Show the sidebar (\u{2303}\u{2318}S)".to_owned()
                    } else {
                        "Hide the sidebar (\u{2303}\u{2318}S)".to_owned()
                    },
                    onclick: move |_| side_hidden.set(!side_hidden()),
                }
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
