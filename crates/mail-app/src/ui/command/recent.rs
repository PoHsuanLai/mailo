//! ⌘K's Recent, with nothing typed: where the person was. The conversations opened in this Space,
//! newest first, then the drafts put aside.
//!
//! This is what the sidebar's Today held. Arc's list of open tabs works because it empties
//! itself; mailo moves nothing by time, so a list that only grew had no place at rest. Here it
//! is one keystroke away and never in the way. What it lists is [`rows`], pure over the store
//! and Today, so a table test can say it.

use super::super::menu::{MenuItem, Right, Tile};
use super::items::{RECENT, avatar_color};
use crate::ui::copy;
use crate::ui::space::SpaceId;
use crate::ui::today::Today;
use ds::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::{DraftId, ThreadId};

/// How many conversations Recent lists. The rest are still a search away.
const SHOWN: usize = 8;

/// The key of a conversation's row.
pub(super) fn thread_key(thread: ThreadId) -> String {
    format!("recent:{thread}")
}

/// The key of a put-aside draft's row.
pub(super) fn draft_key(draft: DraftId) -> String {
    format!("draft:{draft}")
}

/// The conversation a row keyed `rest` (after `recent:`) opens.
pub(super) fn thread_of(rest: &str) -> Option<ThreadId> {
    uuid::Uuid::parse_str(rest).ok().map(ThreadId::from_uuid)
}

/// The draft a row keyed `rest` (after `draft:`) brings back.
pub(super) fn draft_of(rest: &str) -> Option<DraftId> {
    uuid::Uuid::parse_str(rest).ok().map(DraftId::from_uuid)
}

/// Recent's rows: `space`'s opened conversations still in the store, newest first and at most
/// [`SHOWN`], then its drafts put aside.
pub(super) fn rows(store: &SqliteStore, today: &Today, space: SpaceId) -> Vec<MenuItem> {
    let mut opened: Vec<_> = today
        .all(space)
        .into_iter()
        .map(|entry| (entry.item, entry.last_opened))
        .collect();
    opened.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let threads = opened
        .into_iter()
        .filter_map(|(thread, _)| store.thread(thread).ok())
        .take(SHOWN)
        .map(|loaded| {
            let summary = &loaded.summary;
            let who = summary
                .from
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| summary.from.email.clone());
            let name = if summary.subject.trim().is_empty() {
                copy::empty::NO_SUBJECT.to_owned()
            } else {
                summary.subject.clone()
            };
            row(
                thread_key(summary.id),
                name,
                Tile::Avatar {
                    letter: who.chars().next().unwrap_or('·').to_ascii_uppercase(),
                    color: avatar_color(&summary.from.email),
                },
            )
        });
    let drafts = today.parked_in(space).into_iter().map(|parked| {
        row(
            draft_key(parked.item),
            parked.title.clone(),
            Tile::Icon(Icon::Pen),
        )
    });
    threads.chain(drafts).collect()
}

fn row(key: String, name: String, tile: Tile) -> MenuItem {
    MenuItem {
        key,
        tile,
        name,
        help: None,
        right: Right::None,
        group: Some(RECENT.to_owned()),
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
