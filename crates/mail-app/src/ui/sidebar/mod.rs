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
use self::panes::{AccountTiles, PinnedList, PlaceList, counts};
use self::today::TodayList;
use super::switch::{self, Slide};
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::space::edit::Draft;
use crate::today::Today;
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{Glyph, Icon};
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
    rsx! {
        nav {
            class: "side",
            aria_label: "Sidebar",
            onpointerleave: move |_| side_peek.set(false),
            button {
                class: "cmd",
                onclick: move |_| {
                    shell.write().command = Some(String::new());
                },
                Glyph { icon: Icon::Search }
                span { class: "t", "Search or run a command" }
                span { class: "k", "Ctrl T" }
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
                button {
                    class: "space-name",
                    r#type: "button",
                    title: "Edit this Space",
                    aria_label: "Edit the {name} Space",
                    onclick: open_editor,
                    "{name}"
                }
                div { class: "space-dots", role: "group", aria_label: "Spaces",
                    for (index, one) in spaces.read().spaces.iter().enumerate() {
                        {
                            let grad = super::space_editor::gradient_in(&one.look.dots, scheme);
                            let label = one.name.clone();
                            let current = index == space_index;
                            let key = index + 1;
                            rsx! {
                                button {
                                    key: "{index}",
                                    class: "sp",
                                    r#type: "button",
                                    aria_label: "{label} Space",
                                    title: "{label} (Ctrl {key})",
                                    aria_pressed: if current { "true" } else { "false" },
                                    style: "{grad}",
                                    onclick: move |_| {
                                        if editing.read().is_none() {
                                            switch::go(spaces, shell, pages, slide, index);
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                button {
                    class: "foot-btn",
                    r#type: "button",
                    aria_label: "New Space",
                    title: "New Space",
                    onclick: move |_| {
                        if editing.read().is_none() {
                            switch::add(spaces, shell, pages, slide, editing);
                        }
                    },
                    Glyph { icon: Icon::Plus }
                }
                button {
                    class: "foot-btn",
                    r#type: "button",
                    aria_label: "Space settings",
                    title: "Space settings",
                    aria_expanded: if editing.read().is_some() { "true" } else { "false" },
                    onclick: open_editor,
                    Glyph { icon: Icon::Settings }
                }
                button {
                    class: "foot-btn",
                    r#type: "button",
                    aria_label: "Hide sidebar",
                    title: "Hide the sidebar (Ctrl S)",
                    onclick: move |_| {
                        side_peek.set(false);
                        side_hidden.set(!side_hidden());
                    },
                    Glyph { icon: Icon::PanelLeft }
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
