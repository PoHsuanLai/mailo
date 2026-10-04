//! Today: threads opened in this Space, as sidebar tabs that expire.

use super::super::text::sender;
use crate::ui::appearance::WindowDirs;
use crate::ui::today::{IDLE, Today};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::base::time::clock;
use ds::components::app::today_tabs::{TodayTab, TodayTabs};
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::components::lists::section_header::HeaderAction;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::person::PersonSwatch;
use mail_domain::ThreadId;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

/// The Today tabs of `space_index`: each thread opened in it that has not gone idle, with what
/// it has left, on quire's clock. Quire lists a tab while it has time and drops it, by its
/// roster, when it is closed or runs out.
#[component]
pub(super) fn TodayList(
    shell: Signal<Shell>,
    today: Signal<Today>,
    space_index: usize,
    dirs: Option<WindowDirs>,
) -> Element {
    let now = chrono::Utc::now();
    let store = consume_context::<Arc<SqliteStore>>();
    let tabs: Vec<TodayTab<ThreadId>> = today
        .read()
        .entries
        .iter()
        .filter(|entry| entry.space == space_index)
        .enumerate()
        .filter_map(|(index, entry)| {
            let left = left(entry.last_opened, now)?;
            let loaded = store.thread(entry.thread).ok()?;
            let face = today_face(
                initial(&sender(&loaded.summary)),
                AvatarTone::Account(PersonSwatch::nth(index).colour()),
            );
            Some(TodayTab {
                key: entry.thread,
                title: loaded.summary.subject.clone(),
                leading: RowLeading::Avatar(face),
                expires: clock::now() + left,
                common: Common::default(),
                onpointerenter: None,
                onpointerleave: None,
            })
        })
        .collect();
    let live = !tabs.is_empty();
    let selected = shell.read().selected_tab();
    let dirs_close = dirs.clone();
    let dirs_clear = dirs.clone();
    // Nothing in Today, no heading: a source list does not show an empty group.
    let scheduled = !super::super::compose::waiting(&store).is_empty();
    let any = live || scheduled || !today.read().parked(space_index).is_empty();
    rsx! {
        if any {
        SectionHeader {
            title: "Today",
            actions: if live {
                vec![HeaderAction::new("Clear", EventHandler::new(move |()| {
                    today.write().clear(space_index);
                    save(&dirs_clear, &today.read());
                }))]
            } else {
                Vec::new()
            },
        }
        }
        super::super::compose::ParkedDrafts { shell, space_index }
        super::super::compose::ScheduledDrafts { shell }
        TodayTabs::<ThreadId> {
            label: "Today",
            tabs,
            selected,
            onpick: move |id: ThreadId| shell.write().open_from_today(id),
            onclose: move |id: ThreadId| {
                today.write().close(space_index, id);
                save(&dirs_close, &today.read());
            },
        }
    }
}

/// The first character of `name`, upper-cased, or `?` for an empty name.
pub(super) fn initial(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|ch| ch.to_uppercase().next())
        .unwrap_or('?')
}

/// A Today entry's face: one letter on `tone`, the sidebar's rounded square.
fn today_face(initial: char, tone: AvatarTone) -> AvatarFace {
    AvatarFace {
        initial,
        size: AvatarSize::Size16,
        tone,
        shape: AvatarShape::Square,
    }
}

fn save(dirs: &Option<WindowDirs>, today: &Today) {
    if let Some(dirs) = dirs {
        let _ = crate::ui::today::save(&dirs.state, today);
    }
}

/// How long a tab opened at `last_opened` has left at `now`, or `None` when it has gone idle.
fn left(
    last_opened: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    (last_opened + IDLE - now).to_std().ok()
}
