//! History, kept alike by both stores: an open is recorded once per conversation at its newest
//! time, listed newest first, forgotten after ninety days, and survives a reopen.

use crate::support::*;
use chrono::TimeDelta;
use mail_domain::*;
use mail_store::{OPENED_KEPT, SqliteStore, Store};

fn day(days: i64) -> i64 {
    days * 86_400
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

/// Both stores over the same four conversations, one read message each.
fn both(dir: &std::path::Path) -> (SqliteStore, mail_store::MemoryStore) {
    both_on_disk(
        dir,
        (1..=4).map(|n| Message {
            thread: thread(n),
            date: at(0),
            read: ReadState::Read,
            ..message(0x9000 + n)
        }),
    )
}

/// The History list, by thread number.
fn listed(store: &dyn Store) -> Vec<u128> {
    store
        .opened()
        .unwrap()
        .into_iter()
        .map(|summary| summary.id.as_uuid().as_u128() - 0x7000)
        .collect()
}

#[test]
fn both_stores_list_the_newest_open_first_and_each_conversation_once() {
    let dir = tempfile::tempdir().unwrap();
    let (sqlite, memory) = both(dir.path());
    let stores: [(&str, &dyn Store); 2] = [("sqlite", &sqlite), ("memory", &memory)];
    for (name, store) in stores {
        assert_eq!(
            listed(store),
            Vec::<u128>::new(),
            "{name}: nothing opened yet"
        );
        store.record_opened(thread(1), at(10)).unwrap();
        store.record_opened(thread(2), at(20)).unwrap();
        store.record_opened(thread(3), at(30)).unwrap();
        assert_eq!(listed(store), [3, 2, 1], "{name}: newest first");
        store.record_opened(thread(1), at(40)).unwrap();
        assert_eq!(
            listed(store),
            [1, 3, 2],
            "{name}: opened again, it moves up and is not doubled"
        );
        store.record_opened(thread(2), at(5)).unwrap();
        assert_eq!(
            listed(store),
            [1, 3, 2],
            "{name}: an older time than the kept one changes nothing"
        );
        store.record_opened(thread(4), at(40)).unwrap();
        assert_eq!(
            listed(store),
            [1, 4, 3, 2],
            "{name}: a tie goes by thread id"
        );
    }
}

#[test]
fn both_stores_forget_what_was_opened_over_ninety_days_before_the_latest_open() {
    assert_eq!(OPENED_KEPT, TimeDelta::days(90));
    let dir = tempfile::tempdir().unwrap();
    let (sqlite, memory) = both(dir.path());
    let stores: [(&str, &dyn Store); 2] = [("sqlite", &sqlite), ("memory", &memory)];
    for (name, store) in stores {
        store.record_opened(thread(1), at(0)).unwrap();
        store.record_opened(thread(2), at(day(30))).unwrap();
        store.record_opened(thread(3), at(day(89))).unwrap();
        assert_eq!(listed(store), [3, 2, 1], "{name}: all within ninety days");
        // Exactly ninety days after the first: it is on the edge and stays.
        store.record_opened(thread(4), at(day(90))).unwrap();
        assert_eq!(listed(store), [4, 3, 2, 1], "{name}: the edge stays");
        store.record_opened(thread(4), at(day(90) + 1)).unwrap();
        assert_eq!(
            listed(store),
            [4, 3, 2],
            "{name}: one second past, the oldest goes"
        );
        store.record_opened(thread(4), at(day(125))).unwrap();
        assert_eq!(listed(store), [4, 3], "{name}: the next oldest follows");
    }
}

#[test]
fn both_stores_leave_out_a_conversation_they_do_not_hold() {
    let dir = tempfile::tempdir().unwrap();
    let (sqlite, memory) = both(dir.path());
    let stores: [(&str, &dyn Store); 2] = [("sqlite", &sqlite), ("memory", &memory)];
    for (name, store) in stores {
        store.record_opened(thread(99), at(10)).unwrap();
        store.record_opened(thread(1), at(20)).unwrap();
        assert_eq!(listed(store), [1], "{name}");
    }
}

#[test]
fn history_survives_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    {
        let (sqlite, _) = both(dir.path());
        sqlite.record_opened(thread(2), at(10)).unwrap();
        sqlite.record_opened(thread(1), at(20)).unwrap();
    }
    let reopened = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    assert_eq!(listed(&reopened), [1, 2]);
}
