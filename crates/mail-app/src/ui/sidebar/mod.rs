mod folder_act;
mod folder_parts;
mod folder_row;
mod folder_tree;
mod folders;
mod foot;
mod panes;
mod sync;

pub(super) use self::folder_act::folder_places;
use self::folder_tree::{Show, arrange, scope};
use self::folders::FolderList;
pub(in crate::ui) use self::panes::hex_colour;
use self::panes::{Favourites, PlaceList, counts};
use crate::ui::copy;
use crate::ui::space::{Handle, Mail};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::edge_peek::EdgePeek;
use ds::components::app::spaces::{SpaceHead, SpacesFoot};
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName, ExtraClass};
use mail_core::profile::ProfileId;

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
    // Whether a message is waiting for its time on this Space's accounts: the Scheduled place is
    // drawn only then. Read again as a send is scheduled or taken back.
    let desk = try_use_context::<crate::ui::compose::Desk>();
    let scheduled = use_memo(move || {
        let _ = revision();
        if let Some(desk) = desk {
            let _ = desk.outbox.read();
        }
        let store = consume_context::<std::sync::Arc<mail_core::SqliteStore>>();
        !crate::ui::compose::scheduled(&store, &in_scope()).is_empty()
    });
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
    let counts_now = counted.read().clone();
    let pinned = if side_hidden() {
        Shown::Hidden
    } else {
        Shown::Visible
    };
    // With one Space there is no Space to tell apart: no name heads the sidebar and no dot
    // stands in the foot (quire's `SpacesFoot` draws none for one). Its menu is a right click on
    // the foot, wherever it lands that is nobody else's.
    let several = spaces.read().count() > 1;
    rsx! {
        // Hidden, the sidebar floats over the card from the edge while the pointer is there
        // (quire's `EdgePeek`), and a click on the edge pins it again.
        EdgePeek {
            label: copy::aria::SIDEBAR,
            pinned,
            onpin: move |()| side_hidden.set(false),
            common: Common { extra_class: ExtraClass::parse("side").ok(), ..Common::default() },
            // With several Spaces the current one's name heads the sidebar, as a browser's does;
            // a right click on it is the Space's menu, and a plain press does nothing. Downloads
            // stands at the row's right end, its list opening below it.
            div { class: "side-head",
                div { class: "side-title",
                    if several {
                        SpaceHead { handle }
                    }
                }
                crate::ui::downloads::DownloadsButton { shell }
            }
            div { class: "slide",
                // A sign-in in the browser, while it waits: a quiet row at the top.
                crate::ui::add_account::PendingSignIn {}
                Favourites { shell, pages, space: space.clone(), counted: counts_now }
                PlaceList { shell, pages, badges, folded, scheduled: scheduled() }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, revision, section, show }
                }
            }
            // Sync on the left, the Space dots once there are several, and one chevron on the
            // right that opens Tidy Up…, Settings and the sidebar's toggle (`foot::MoreMenu`). The foot's
            // own `+` is left out: the Space's menu holds New Space. Sync's mark says how fresh
            // the mail is, and warns when it is not (`sync`). Each end is a hold of mailo's own,
            // as wide as the other (`style/shell.css`), so the dots stand at the foot's centre
            // whatever either end holds.
            div {
                oncontextmenu: move |event: MouseEvent| {
                    event.prevent_default();
                    let at = event.client_coordinates();
                    handle.open_menu(space_id, Point { x: Px(at.x as f32), y: Px(at.y as f32) });
                },
                SpacesFoot {
                    handle,
                    new_payload: move |()| Mail::new_in(&ProfileId::default()),
                    new_space: NewSpace::Hidden,
                    leading: rsx! {
                        div { class: "foot-lead",
                            sync::SyncMark { shell, revision }
                        }
                    },
                    trailing: rsx! {
                        div { class: "foot-end",
                            foot::MoreMenu { shell, side_hidden }
                        }
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
