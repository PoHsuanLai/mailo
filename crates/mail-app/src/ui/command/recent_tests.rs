use super::{SHOWN, draft_key, draft_of, rows, thread_key, thread_of};
use crate::ui::fixtures::{inbox_query, seeded, work};
use crate::ui::space::SpaceId;
use crate::ui::today::{Today, at};
use chrono::{TimeZone, Utc};
use mail_core::Store;
use mail_domain::{DraftId, ThreadId};

fn time(minute: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + minute * 60, 0).unwrap()
}

/// What was opened last comes first, the drafts put aside after, and only this Space's.
#[test]
fn recent_lists_this_spaces_conversations_newest_first_then_its_drafts() {
    let built = work();
    let store = built.store.clone();
    let threads: Vec<ThreadId> = store
        .threads(&inbox_query(), Utc::now())
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    let (older, newer, elsewhere) = (threads[0], threads[1], threads[2]);
    let draft = DraftId::generate();
    let mut today = Today::default();
    today.opened(SpaceId(0), older, at(time(0)));
    today.opened(SpaceId(0), newer, at(time(5)));
    today.opened(SpaceId(1), elsewhere, at(time(9)));
    today.park(SpaceId(0), draft, "Plans", at(time(7)));

    let keys: Vec<String> = rows(&store, &today, SpaceId(0))
        .into_iter()
        .map(|row| row.key)
        .collect();
    assert_eq!(
        keys,
        [thread_key(newer), thread_key(older), draft_key(draft)]
    );
}

/// Nothing opened and nothing put aside: Recent lists nothing of its own.
#[test]
fn recent_is_empty_with_nothing_opened() {
    let (store, _dir) = seeded();
    assert!(rows(&store, &Today::default(), SpaceId(0)).is_empty());
}

/// However much was opened, Recent lists a handful; the rest are a search away.
#[test]
fn recent_lists_at_most_a_handful_of_conversations() {
    let built = work();
    let store = built.store.clone();
    let threads: Vec<ThreadId> = store
        .threads(&inbox_query(), Utc::now())
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert!(threads.len() > SHOWN, "the fixture has too few threads");
    let mut today = Today::default();
    for (minute, thread) in threads.iter().enumerate() {
        today.opened(SpaceId(0), *thread, at(time(minute as i64)));
    }
    assert_eq!(rows(&store, &today, SpaceId(0)).len(), SHOWN);
}

/// A row's key reads back as what it stands for.
#[test]
fn recent_keys_read_back() {
    let thread = ThreadId::generate();
    let draft = DraftId::generate();
    let key = thread_key(thread);
    assert_eq!(
        thread_of(key.strip_prefix("recent:").unwrap()),
        Some(thread)
    );
    let key = draft_key(draft);
    assert_eq!(draft_of(key.strip_prefix("draft:").unwrap()), Some(draft));
    assert_eq!(thread_of("not an id"), None);
}
