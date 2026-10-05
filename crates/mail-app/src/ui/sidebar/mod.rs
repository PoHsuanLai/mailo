mod folder_act;
mod folder_parts;
mod folder_row;
mod folder_tree;
mod folders;
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
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::command_pill::CommandPill;
use ds::components::app::edge_peek::EdgePeek;
use ds::components::controls::button_model::ImagePosition;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName, ExtraClass};
use ds::style::space::frame_vars::FrameVars;
use ds::style::tokens::control_size::ControlSize;

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
    let open_editor = move |_| {
        if editing.read().is_none() {
            switch::edit(spaces, editing);
        }
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
            CommandPill {
                label: "Search or run a command".to_owned(),
                shortcut: Shortcut(vec![ShortcutKey::Super, ShortcutKey::Char('k')]),
                onclick: move |()| {
                    shell.write().command = Some(String::new());
                },
            }
            div { class: "slide",
                AccountTiles { shell, pages, space: space.clone(), counted: tiles.clone() }
                PlaceList { shell, pages, badges, folded }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, revision, section, show }
                }
                PinnedList { shell, pages, space: space.clone(), pins: tiles.pins.clone() }
                TodayList { shell, today, space_index, dirs: dirs.clone() }
            }
            div { class: "side-foot",
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Inline,
                    label: name.clone(),
                    title: "Edit this Space".to_owned(),
                    common: Common {
                        aria_label: Some(format!("Edit the {name} Space")),
                        extra_class: ExtraClass::parse("space-name").ok(),
                        ..Common::default()
                    },
                    onclick: open_editor,
                }
                div { class: "space-dots", role: "group", aria_label: "Spaces",
                    for (index, one) in spaces.read().spaces.iter().enumerate() {
                        {
                            let selection = Selection::of(&index, &space_index);
                            // ⌘ and the Space's place, one to nine: the keys `App` switches on.
                            let keys = char::from_digit(u32::try_from(index + 1).unwrap_or(0), 10)
                                .map_or_else(Vec::new, |digit| vec![ShortcutKey::Super, ShortcutKey::Char(digit)]);
                            rsx! {
                                SpaceDot {
                                    key: "{index}",
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
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    size: ControlSize::Small,
                    icon: Icon::Plus,
                    label: "New Space",
                    title: "New Space".to_owned(),
                    onclick: move |_| {
                        if editing.read().is_none() {
                            switch::add(spaces, shell, pages, editing);
                        }
                    },
                }
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    size: ControlSize::Small,
                    icon: Icon::Settings,
                    label: "Space settings",
                    title: "Space settings".to_owned(),
                    shown: if editing.read().is_some() { Shown::Visible } else { Shown::Hidden },
                    onclick: move |_| {
                        if editing.read().is_none() {
                            switch::edit(spaces, editing);
                        }
                    },
                }
                Button {
                    bezel: ds::components::controls::button_model::Bezel::Toolbar,
                    image: ImagePosition::Only,
                    size: ControlSize::Small,
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
