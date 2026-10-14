//! Today, at the top of the sidebar: the conversations opened in this Space and the drafts put
//! aside, with the messages waiting for their time under them.
//!
//! Nothing leaves Today by time. An entry stays until Clear Today, in the section's right-click
//! menu, which forgets the shortcuts and does not touch the mail. A parked draft stays until it
//! is reopened, sent or discarded, and a scheduled message until it leaves or is taken back. What
//! the section lists is data ([`gather`], [`rows`]) and pure, so a table test can say it.

use super::panes::{face, initial, titled};
use super::tagged;
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::{Desk, cancel_waiting, reopen, waiting};
use crate::ui::copy;
use crate::ui::menus::when_words;
use crate::ui::space::SpaceId;
use crate::ui::text::sender;
use crate::ui::today::Today;
use crate::ui::view::Shell;
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
    /// The messages waiting for their time, soonest first: draft, title and when.
    pub scheduled: Vec<(DraftId, String, String)>,
}

/// A row of the section, by what it stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum TodayKey {
    Head,
    Thread(ThreadId),
    Draft(DraftId),
    Scheduled(DraftId),
}

/// What the section's right-click menu asks for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TodayPick {
    /// Forget this Space's Today conversations.
    Clear,
    /// Take a message waiting for its time back.
    CancelSend(DraftId),
}

/// The section's rows in order, as (key, words, glyph or face): the conversations, the drafts,
/// then the messages waiting, with what each says beside it.
pub(super) fn rows(gathered: &Gathered) -> Vec<(TodayKey, String, RowLeading, Option<String>)> {
    let opened = gathered.today.iter().map(|recent| {
        (
            TodayKey::Thread(recent.thread),
            recent.title.clone(),
            RowLeading::Avatar(recent.face),
            None,
        )
    });
    let parked = gathered.parked.iter().map(|(draft, title)| {
        (
            TodayKey::Draft(*draft),
            title.clone(),
            RowLeading::Icon(Icon::Pen),
            None,
        )
    });
    let scheduled = gathered.scheduled.iter().map(|(draft, title, when)| {
        (
            TodayKey::Scheduled(*draft),
            title.clone(),
            RowLeading::Icon(Icon::Clock),
            Some(when.clone()),
        )
    });
    opened.chain(parked).chain(scheduled).collect()
}

/// The right-click menu over `key`: Cancel Sending on a message waiting for its time, and Clear
/// Today while there is a conversation to clear.
pub(super) fn menu_for(key: &TodayKey, gathered: &Gathered) -> Vec<MenuItem<TodayPick>> {
    let mut out = Vec::new();
    if let TodayKey::Scheduled(draft) = key
        && let Some((_, title, _)) = gathered.scheduled.iter().find(|(at, _, _)| at == draft)
    {
        out.push(
            MenuItem::new(
                TodayPick::CancelSend(*draft),
                copy::menus::cancel_sending(title),
            )
            .with_image(MenuImage::Icon(Icon::Clock)),
        );
    }
    if !gathered.today.is_empty() {
        out.push(
            MenuItem::new(TodayPick::Clear, copy::menus::CLEAR_TODAY)
                .with_image(MenuImage::Icon(Icon::X)),
        );
    }
    out
}

/// `opened` Today conversations, newest first. The kit lists them in the order they were
/// opened, but a file edited by hand need not be.
pub(super) fn newest_first(mut opened: Vec<(ThreadId, Epoch)>) -> Vec<ThreadId> {
    opened.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    opened.into_iter().map(|(thread, _)| thread).collect()
}

/// What the section lists now: every one of `space`'s Today conversations still in the store,
/// however long ago it was opened, its parked drafts, and the messages waiting for their time.
pub(super) fn gather(store: &SqliteStore, today: &Today, space: SpaceId) -> Gathered {
    let now = chrono::Utc::now();
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
        scheduled: waiting(store)
            .into_iter()
            .map(|one| {
                (
                    one.draft,
                    one.title,
                    when_words(one.at, now, &chrono::Local),
                )
            })
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
    // Read so a send scheduled or taken back, and a conversation archived, are drawn.
    if let Some(desk) = desk {
        let _ = desk.outbox.read();
    }
    let _ = revision();
    let store = consume_context::<Arc<SqliteStore>>();
    let gathered = gather(&store, &today.read(), space);
    let shown = rows(&gathered);
    if shown.is_empty() {
        return rsx! {};
    }
    let open = shell.read().selected_tab();
    let mut items = vec![ListItem::heading(
        TodayKey::Head,
        rsx! { SectionHeader { title: copy::headers::TODAY } },
    )];
    for (key, title, leading, when) in shown {
        let on = matches!(key, TodayKey::Thread(thread) if open == Some(thread));
        let pressed = key.clone();
        let content = rsx! {
            Row {
                leading,
                title: title.clone(),
                common: tagged("today", title.clone()),
                accessory: when.map_or(Accessory::None, Accessory::Text),
                state: RowState {
                    selection: if on { Selection::Selected } else { Selection::Unselected },
                    ..RowState::default()
                },
                onclick: move |press: Press| {
                    // A right click is the section's menu. So is a click on a message waiting
                    // for its time: taking it back is said in words before it is done.
                    let asks = press.button == PointerButton::Secondary
                        || matches!(pressed, TodayKey::Scheduled(_));
                    if asks {
                        menu.set(Some((pressed.clone(), press.at)));
                        return;
                    }
                    match &pressed {
                        TodayKey::Thread(thread) => shell.write().open_from_today(*thread),
                        TodayKey::Draft(draft) => {
                            if let Some(desk) = desk {
                                reopen(desk, shell, *draft);
                            }
                        }
                        TodayKey::Head | TodayKey::Scheduled(_) => {}
                    }
                },
            }
        };
        items.push(ListItem::row(key, title, content));
    }
    let asked = menu().map(|(key, at)| (menu_for(&key, &gathered), at));
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
                        TodayPick::CancelSend(draft) => {
                            if let Some(desk) = desk {
                                // A refusal is said in the toast; the message stays as it was.
                                let _ = cancel_waiting(desk, shell, draft);
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
