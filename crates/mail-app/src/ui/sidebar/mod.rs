//! The sidebar on the Space's colour.
//!
//! Account tiles, places, labels, pinned people, Today, and the foot. Writing and
//! fetching moved into the list bar; how the Space looks is the editor's, opened from
//! the foot.

mod folder_act;
mod folder_parts;
mod folder_row;
mod folder_tree;
mod folders;
mod panes;
mod today;

pub(super) use self::folder_act::folder_places;
use self::folder_tree::{Show, arrange, scope};
use self::folders::FolderList;
pub(in crate::ui) use self::panes::hex_colour;
use self::panes::{AccountTiles, PinnedList, PlaceList, counts};
use self::today::TodayList;
pub(in crate::ui) use self::today::{initial, today_face};
use super::switch::{self, Slide};
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::space::edit::Draft;
use crate::today::Today;
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{
    CommandPill, FrameVars, Here, Icon, IconButton, IconButtonVariant, Key, Shortcut, SideState,
    SpaceDot, Switch,
};
use mail_domain::ThreadId;

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
    side_peek: Signal<bool>,
    just_added: Signal<Option<ThreadId>>,
    editing: Signal<Option<Draft>>,
    slide: Signal<Option<Slide>>,
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
    let scheme = ds::use_env().scheme;
    let tiles = counted.read().clone();
    let slide_class = slide().map_or("slide", Slide::class);
    let name = space.name.clone();
    let open_editor = move |_| {
        if editing.read().is_none() {
            switch::edit(spaces, editing);
        }
    };
    // Hidden, the sidebar is out of the grid; peeking, it floats over the card from the edge.
    let side = match (side_hidden(), side_peek()) {
        (false, _) => SideState::Shown,
        (true, false) => SideState::Hidden,
        (true, true) => SideState::Peek,
    };
    rsx! {
        nav {
            class: "side ds-side",
            "data-side": side.slug(),
            aria_label: "Sidebar",
            onpointerleave: move |_| side_peek.set(false),
            CommandPill {
                label: "Search or run a command".to_owned(),
                shortcut: Shortcut(vec![Key::Ctrl, Key::Char('t')]),
                onclick: move |()| {
                    shell.write().command = Some(String::new());
                },
            }
            // Keyed by the Space, so a switch mounts it afresh and the slide plays each time.
            div { key: "{space_index}", class: "{slide_class}",
                AccountTiles { shell, pages, space: space.clone(), counted: tiles.clone() }
                PlaceList { shell, pages, badges, folded }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, revision, section, show }
                }
                PinnedList { shell, pages, space: space.clone(), pins: tiles.pins.clone() }
                TodayList { shell, today, space_index, dirs: dirs.clone(), just_added }
            }
            div { class: "side-foot",
                // quire's frame word, in mailo's box that gives it the foot's free width.
                span { class: "space-name",
                    ds::Button {
                        variant: ds::ButtonVariant::Frame,
                        label: name.clone(),
                        title: "Edit this Space".to_owned(),
                        aria_label: format!("Edit the {name} Space"),
                        onclick: open_editor,
                    }
                }
                div { class: "space-dots", role: "group", aria_label: "Spaces",
                    for (index, one) in spaces.read().spaces.iter().enumerate() {
                        {
                            let here = if index == space_index { Here::Current } else { Here::Elsewhere };
                            // Ctrl and the Space's place, one to nine: the keys `App` switches on.
                            let keys = char::from_digit(u32::try_from(index + 1).unwrap_or(0), 10)
                                .map_or_else(Vec::new, |digit| vec![Key::Ctrl, Key::Char(digit)]);
                            rsx! {
                                SpaceDot {
                                    key: "{index}",
                                    name: one.name.clone(),
                                    frame: FrameVars::of(&one.look, scheme),
                                    here,
                                    shortcut: Shortcut(keys),
                                    onclick: move |()| {
                                        if editing.read().is_none() {
                                            switch::go(spaces, shell, pages, slide, index);
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                IconButton {
                    variant: IconButtonVariant::Foot,
                    icon: Icon::Plus,
                    label: "New Space".to_owned(),
                    tooltip: "New Space".to_owned(),
                    onclick: move |_| {
                        if editing.read().is_none() {
                            switch::add(spaces, shell, pages, slide, editing);
                        }
                    },
                }
                IconButton {
                    variant: IconButtonVariant::Foot,
                    icon: Icon::Settings,
                    label: "Space settings".to_owned(),
                    tooltip: "Space settings".to_owned(),
                    expanded: if editing.read().is_some() { Switch::On } else { Switch::Off },
                    onclick: move |_| {
                        if editing.read().is_none() {
                            switch::edit(spaces, editing);
                        }
                    },
                }
                IconButton {
                    variant: IconButtonVariant::Foot,
                    icon: Icon::PanelLeft,
                    label: "Hide sidebar".to_owned(),
                    tooltip: "Hide the sidebar (Ctrl S)".to_owned(),
                    onclick: move |_| {
                        side_peek.set(false);
                        side_hidden.set(!side_hidden());
                    },
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
