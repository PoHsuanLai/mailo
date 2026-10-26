mod folder_act;
mod folder_parts;
mod folder_row;
mod folder_tree;
mod folders;
mod foot;
mod join;
mod lit;
mod panes;
mod sync;
mod tile_menu;
mod tiles;

pub(super) use self::folder_act::{folder_places, known_labels};
use self::folder_tree::{Show, arrange, scope};
use self::folders::FolderList;
use self::panes::{Favourites, PlaceList, counts};
pub(in crate::ui) use self::panes::{hex_colour, place_icon};
use self::tiles::{AccountTiles, tile_filter, tile_rows};
use crate::ui::copy;
use crate::ui::live::use_live;
use crate::ui::space::{Handle, Mail};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::edge_peek::EdgePeek;
use ds::components::app::spaces::{SpaceHead, SpacesFoot};
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName, ExtraClass};
use mail_client::profile::ProfileId;

/// A `data-<name>` of mailo's own on a quire component: where a hover card or a test finds the
/// thing the element stands for.
pub(super) fn tagged(name: &'static str, value: impl Into<String>) -> Common {
    Common {
        data: DataName::parse(name)
            .map(|name| vec![DataAttr::new(name, value)])
            .unwrap_or_default(),
        ..Common::default()
    }
}

/// Go to the place at `index`, from the first page of its list: the one way a click, a ⌘K pick
/// or a drop of the sidebar's own rows turns the window to another place.
pub(in crate::ui) fn go_to(mut shell: Signal<Shell>, mut pages: Signal<u32>, index: usize) {
    shell.write().select(index);
    pages.set(1);
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
    handle: Handle,
    side_hidden: Signal<bool>,
) -> Element {
    let spaces = handle.spaces();
    let space = spaces.read().current().clone();
    let counted = use_memo(move || {
        let _ = crate::ui::live::moved();
        let client = crate::ui::live::client();
        counts(&client, spaces.read().current())
    });
    // Every account with its folders and labels, kept read by the client.
    let accounts = use_live(|| mail_client::query::Accounts);
    // Each tile's unread, counted as the client counts anything: read again as mail arrives.
    let tile_counts = use_live(move || {
        mail_client::query::Counts(
            spaces
                .read()
                .current()
                .payload
                .accounts
                .iter()
                .cloned()
                .map(tile_filter)
                .collect(),
        )
    });
    // The Space's account tiles, as the Space's accounts change or their counts do.
    let tiled = use_memo(move || {
        let accounts = accounts.read();
        let counts = tile_counts.read();
        tile_rows(
            accounts
                .value()
                .map_or(&[][..], |accounts| accounts.as_slice()),
            counts.value().map(|counted| counted.as_ref()),
            spaces.read().current(),
        )
    });
    // The accounts the Folders and Labels sections are for. A memo, so a keystroke in the search
    // box — a shell change — lays out neither again.
    let in_scope = use_memo(move || scope(&shell.read()));
    // The labels drawn: only those of the accounts in scope.
    let drawn = use_memo(move || {
        accounts
            .read()
            .value()
            .map(|accounts| folder_act::labels_in(accounts, &in_scope()))
            .unwrap_or_default()
    });
    // Whether a message is waiting for its time on this Space's accounts: the Scheduled place is
    // drawn only then. Read again as a send is scheduled or taken back.
    let desk = try_use_context::<crate::ui::compose::Desk>();
    let scheduled = use_memo(move || {
        let _ = crate::ui::live::moved();
        if let Some(desk) = desk {
            let _ = desk.outbox.read();
        }
        let client = crate::ui::live::client();
        !crate::ui::compose::scheduled(&client, &in_scope()).is_empty()
    });
    let show = use_signal(|| Show::Followed);
    let folders = use_memo(move || {
        let accounts = accounts.read();
        let accounts = accounts
            .value()
            .map_or(&[][..], |accounts| accounts.as_slice());
        arrange(&folder_act::load(accounts, &in_scope()), show())
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
    // The current Space's name heads the sidebar, one Space or several: it says where you are.
    // The dots stand in the foot only once there are several to tell apart (quire's `SpacesFoot`
    // draws none for one).
    //
    // A right click anywhere in the sidebar is the Space's menu, unless what it landed on has a
    // menu of its own: an account tile, a folder, a Space's dot. Each of those takes the click
    // (its `prevent_default`) before it reaches the head, the list or the foot.
    let space_menu = move |event: MouseEvent| {
        if !event.default_action_enabled() {
            return;
        }
        event.prevent_default();
        let at = event.client_coordinates();
        handle.open_menu(
            space_id,
            Point {
                x: Px(at.x as f32),
                y: Px(at.y as f32),
            },
        );
    };
    rsx! {
        // Hidden, the sidebar floats over the card from the edge while the pointer is there
        // (quire's `EdgePeek`), and a click on the edge pins it again.
        EdgePeek {
            label: copy::aria::SIDEBAR,
            pinned,
            onpin: move |()| side_hidden.set(false),
            common: Common { extra_class: ExtraClass::parse("side").ok(), ..Common::default() },
            // The current Space's name heads the sidebar, as a browser's does; a plain press on it
            // does nothing. Downloads stands at the row's right end, its list opening below it.
            div { class: "side-head", oncontextmenu: space_menu,
                div { class: "side-title",
                    SpaceHead { handle }
                }
                crate::ui::downloads::DownloadsButton { shell }
            }
            div { class: "slide", oncontextmenu: space_menu,
                AccountTiles { shell, pages, handle, rows: tiled() }
                Favourites { shell, pages, space: space.clone(), counted: counts_now }
                PlaceList { shell, pages, badges, folded, drawn: drawn(), scheduled: scheduled() }
                if let Some(section) = folders() {
                    FolderList { shell, pages, badges, section, show }
                }
            }
            // Sync on the left, the Space dots once there are several, and one chevron on the
            // right that opens Tidy Up…, Settings and the sidebar's toggle (`foot::MoreMenu`). The foot's
            // own `+` is left out: the Space's menu holds New Space. Sync's mark says how fresh
            // the mail is, and warns when it is not (`sync`). Each end is a hold of mailo's own,
            // as wide as the other (`style/shell.css`), so the dots stand at the foot's centre
            // whatever either end holds.
            div { oncontextmenu: space_menu,
                SpacesFoot {
                    handle,
                    new_payload: move |()| Mail::new_in(&ProfileId::default()),
                    new_space: NewSpace::Hidden,
                    leading: rsx! {
                        div { class: "foot-lead",
                            sync::SyncMark { shell }
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
mod client_tests;
#[cfg(test)]
mod folder_place_tests;
#[cfg(test)]
mod folder_store_tests;
#[cfg(test)]
mod folder_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;
