//! ⌘K's Recent, with nothing typed: where the person was. The conversations opened in this Space,
//! newest first, then the drafts put aside, each named by what is in it and when it was last
//! touched.
//!
//! This is what the sidebar's Today held. Arc's list of open tabs works because it empties
//! itself; mailo moves nothing by time, so a list that only grew had no place at rest. Here it
//! is one keystroke away and never in the way. What it lists is [`rows`], pure over the store
//! and Today, so a table test can say it.

use super::super::menu::{MenuItem, Tile};
use super::items::{RECENT, avatar_color};
use crate::ui::copy;
use crate::ui::space::SpaceId;
use crate::ui::today::Today;
use chrono::{DateTime, Utc};
use ds::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::{Draft, DraftId, ThreadId};

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
/// [`SHOWN`], then its drafts put aside that are still in the store with something in them.
pub(super) fn rows(
    store: &SqliteStore,
    today: &Today,
    space: SpaceId,
    now: DateTime<Utc>,
    clock: mail_core::when::Clock,
) -> Vec<MenuItem> {
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
            let who = summary.from.shown().to_owned();
            let name = mail_domain::subject_or_none(&summary.subject).into_owned();
            row(
                thread_key(summary.id),
                name,
                Tile::Avatar {
                    letter: who.chars().next().unwrap_or('·').to_ascii_uppercase(),
                    color: avatar_color(&summary.from.email),
                },
            )
        });
    // A draft sent, discarded or emptied somewhere else leaves no row behind.
    let drafts = today
        .parked_in(space)
        .into_iter()
        .filter_map(|parked| store.draft(parked.item).ok())
        .filter(|draft| !draft.untouched())
        .map(|draft| MenuItem {
            help: Some(copy::status::draft_touched(&mail_core::when::listed(
                draft.updated,
                now,
                &chrono::Local,
                clock,
            ))),
            ..row(
                draft_key(draft.id),
                draft_name(&draft),
                Tile::Icon(Icon::Pen),
            )
        });
    threads.chain(drafts).collect()
}

/// The most characters of a draft's first line its row is named by, when it has no subject and
/// nobody to go to.
const FIRST_WORDS: usize = 40;

/// What a draft's row is called: its subject; else who it goes to; else its first words.
fn draft_name(draft: &Draft) -> String {
    if !draft.subject.trim().is_empty() {
        return draft.subject.trim().to_owned();
    }
    if let Some(to) = draft.to.iter().chain(&draft.cc).chain(&draft.bcc).next() {
        return copy::status::draft_to(to.shown());
    }
    let first = draft.written().lines().next().unwrap_or_default().trim();
    if first.is_empty() {
        return mail_domain::NO_SUBJECT.to_owned();
    }
    let mut words: String = first.chars().take(FIRST_WORDS).collect();
    if first.chars().count() > FIRST_WORDS {
        words.push('\u{2026}');
    }
    words
}

fn row(key: String, name: String, tile: Tile) -> MenuItem {
    MenuItem {
        tile,
        group: Some(RECENT.to_owned()),
        ..MenuItem::plain(key, name)
    }
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
