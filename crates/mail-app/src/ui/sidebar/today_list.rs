//! Today, at the top of the sidebar: the conversations opened in this Space and the drafts put
//! aside. A message waiting for its time is in the Scheduled place, not here.
//!
//! Nothing leaves Today by time. An entry stays until Clear Today, in the right-click menu of
//! Today's own heading, which forgets the shortcuts and does not touch the mail; a row's right
//! click offers nothing that clears the whole section. A parked draft stays until it is reopened,
//! sent or discarded. What the section lists is data ([`gather`], [`rows`]) and pure, so a table
//! test can say it.

use super::panes::{face, initial, titled};
use super::tagged;
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::{Desk, reopen};
use crate::ui::copy;
use crate::ui::space::SpaceId;
use crate::ui::text::sender;
use crate::ui::today::Today;
use crate::ui::view::{Shell, SideList};
use dioxus::prelude::*;
use ds::base::press::{PointerButton, Press};
use ds::base::vocab::RowState;
use ds::components::app::spaces::Epoch;
use ds::components::content::avatar::AvatarFace;
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::host::measure::Anchor;
use ds::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::{DraftId, ThreadId};
use std::sync::Arc;

/// One Today conversation as the section draws it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Recent {
    pub thread: ThreadId,
    pub title: String,
    pub face: AvatarFace,
}

/// Everything the section lists, gathered.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Gathered {
    /// This Space's Today conversations, newest first.
    pub today: Vec<Recent>,
    /// The drafts put aside in this Space, newest first, by title.
    pub parked: Vec<(DraftId, String)>,
}

/// A row of the section, by what it stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum TodayKey {
    Head,
    Thread(ThreadId),
    Draft(DraftId),
}

/// What Today's right-click menu asks for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TodayPick {
    /// Forget this Space's Today conversations.
    Clear,
}

/// The section's rows in order, as (key, words, glyph or face): the conversations, then the
/// drafts.
pub(super) fn rows(gathered: &Gathered) -> Vec<(TodayKey, String, RowLeading)> {
    let opened = gathered.today.iter().map(|recent| {
        (
            TodayKey::Thread(recent.thread),
            recent.title.clone(),
            RowLeading::Avatar(recent.face),
        )
    });
    let parked = gathered.parked.iter().map(|(draft, title)| {
        (
            TodayKey::Draft(*draft),
            title.clone(),
            RowLeading::Icon(Icon::Pen),
        )
    });
    opened.chain(parked).collect()
}

/// The right-click menu over `key`: on Today's heading, Clear Today while there is a
/// conversation to clear; on a row, nothing. Clearing the whole section is never a row's to
/// offer.
pub(super) fn menu_for(key: &TodayKey, gathered: &Gathered) -> Vec<MenuItem<TodayPick>> {
    match key {
        TodayKey::Head if !gathered.today.is_empty() => vec![
            MenuItem::new(TodayPick::Clear, copy::menus::CLEAR_TODAY)
                .with_image(MenuImage::Icon(Icon::X)),
        ],
        _ => Vec::new(),
    }
}

/// `opened` Today conversations, newest first. The kit lists them in the order they were
/// opened, but a file edited by hand need not be.
pub(super) fn newest_first(mut opened: Vec<(ThreadId, Epoch)>) -> Vec<ThreadId> {
    opened.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    opened.into_iter().map(|(thread, _)| thread).collect()
}

/// What the section lists now: every one of `space`'s Today conversations still in the store,
/// however long ago it was opened, and its parked drafts.
pub(super) fn gather(store: &SqliteStore, today: &Today, space: SpaceId) -> Gathered {
    let opened = today
        .all(space)
        .into_iter()
        .map(|entry| (entry.item, entry.last_opened))
        .collect();
    let recent = newest_first(opened)
        .into_iter()
        .enumerate()
        .filter_map(|(index, thread)| {
            let loaded = store.thread(thread).ok()?;
            Some(Recent {
                thread,
                title: titled(&loaded.summary.subject),
                face: face(initial(&sender(&loaded.summary)), index),
            })
        })
        .collect();
    Gathered {
        today: recent,
        parked: today
            .parked_in(space)
            .into_iter()
            .map(|parked| (parked.item, parked.title.clone()))
            .collect(),
    }
}

/// The Today section: nothing at all while it holds nothing.
#[component]
pub(in crate::ui) fn TodayList(
    shell: Signal<Shell>,
    today: Signal<Today>,
    space: SpaceId,
    revision: Signal<u64>,
    dirs: Option<WindowDirs>,
) -> Element {
    let desk = try_use_context::<Desk>();
    let mut menu = use_signal(|| None::<(TodayKey, Point)>);
    // Gathered again only when what it reads changes: Today, the Space, a conversation archived
    // (the revision). Not on every write to the shell, which a keystroke in the search box is.
    let gathered = use_memo(use_reactive!(|space| {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        gather(&store, &today.read(), space)
    }));
    // Only which Today row is open is read from the shell.
    let open = use_memo(move || shell.read().selected_in(SideList::Today));
    let shown = rows(&gathered.read());
    if shown.is_empty() {
        return rsx! {};
    }
    let open = open();
    // Where a test finds Today's heading.
    let section = "today";
    let mut items = vec![ListItem::heading(
        TodayKey::Head,
        rsx! {
            // Today's own menu, Clear Today, is a right click on its heading.
            div { "data-section": section,
                oncontextmenu: move |event: MouseEvent| {
                    event.prevent_default();
                    let at = event.client_coordinates();
                    menu.set(Some((TodayKey::Head, Point { x: Px(at.x as f32), y: Px(at.y as f32) })));
                },
                SectionHeader { title: copy::headers::TODAY }
            }
        },
    )];
    for (key, title, leading) in shown {
        let on = matches!(key, TodayKey::Thread(thread) if open == Some(thread));
        let pressed = key.clone();
        let content = rsx! {
            Row {
                leading,
                title: title.clone(),
                common: tagged("today", title.clone()),
                state: RowState {
                    selection: if on { Selection::Selected } else { Selection::Unselected },
                    ..RowState::default()
                },
                onclick: move |press: Press| {
                    // A row's right click offers nothing: Clear Today is the heading's.
                    if press.button == PointerButton::Secondary {
                        return;
                    }
                    match &pressed {
                        TodayKey::Thread(thread) => {
                            shell.write().open_from(*thread, SideList::Today)
                        }
                        TodayKey::Draft(draft) => {
                            if let Some(desk) = desk {
                                reopen(desk, shell, *draft);
                            }
                        }
                        TodayKey::Head => {}
                    }
                },
            }
        };
        items.push(ListItem::row(key, title, content));
    }
    let asked = menu().map(|(key, at)| (menu_for(&key, &gathered.read()), at));
    rsx! {
        List::<TodayKey> {
            label: copy::aria::TODAY,
            items,
            style: ListStyle::SourceList,
        }
        if let Some((items, at)) = asked.filter(|(items, _)| !items.is_empty()) {
            Menu::<TodayPick> {
                placement: MenuPlacement::default(),
                anchor: Anchor::Point(at),
                items,
                onpick: move |pick: TodayPick| {
                    menu.set(None);
                    match pick {
                        TodayPick::Clear => {
                            today.write().clear(space);
                            if let Some(dirs) = &dirs {
                                let _ = crate::ui::today::save(&dirs.state, &today.read());
                            }
                        }
                    }
                },
                onclose: move |()| menu.set(None),
            }
        }
    }
}

#[cfg(test)]
#[path = "today_list_tests.rs"]
mod tests;
