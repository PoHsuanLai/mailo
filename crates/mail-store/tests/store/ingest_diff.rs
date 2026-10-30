//! A sync hands over the flags of every message in a mailbox, nearly all of them what they were
//! last time. What has not changed is not written, not said, and does not rebuild its summary.

use crate::support::*;
use mail_domain::*;
use mail_store::Store;

/// `count` messages in the inbox, message `n` at UID `n`, each in a thread of its own.
fn delivered(store: &dyn Store, count: u128) -> Vec<Message> {
    (1..=count)
        .map(|n| {
            let m = message(n);
            deliver(store, &m, imap("INBOX", n as u32));
            m
        })
        .collect()
}

/// A sync of the inbox that says every message of `count` is unread and unstarred, except the
/// ones `read` lists, which are read.
fn flags(count: u32, read: &[u32]) -> Ingest {
    Ingest {
        flags: (1..=count)
            .map(|uid| {
                let state = if read.contains(&uid) {
                    ReadState::Read
                } else {
                    ReadState::Unread
                };
                (imap("INBOX", uid), state, Star::Unstarred)
            })
            .collect(),
        ..ingest("INBOX")
    }
}

#[test]
fn flags_the_store_already_holds_are_not_written_again() {
    let found = Both::new().agree(|store| {
        delivered(store, 5);
        let first = store.ingest(acct_account(), flags(5, &[2])).unwrap();
        let second = store.ingest(acct_account(), flags(5, &[2])).unwrap();
        (first.changes.len(), second.changes.len())
    });
    // The first says the one message that was read; the second has nothing to say.
    assert_eq!(found, (1, 0));
}

#[test]
fn a_sync_that_finds_nothing_new_commits_nothing() {
    let store = sqlite();
    delivered(&store, 5);
    store.ingest(acct_account(), flags(5, &[2])).unwrap();
    let rx = store.subscribe();
    let before = store.raw_connection().total_changes();

    store.ingest(acct_account(), flags(5, &[2])).unwrap();

    let said: Vec<_> = rx.try_iter().collect();
    assert!(said.is_empty(), "{said:?}");
    assert_eq!(store.raw_connection().total_changes(), before);
}

#[test]
fn one_flag_flipped_among_many_changes_that_message_and_its_conversation_only() {
    let found = Both::new().agree(|store| {
        let all = delivered(store, 6);
        store.ingest(acct_account(), flags(6, &[])).unwrap();
        let patch = store.ingest(acct_account(), flags(6, &[4])).unwrap();
        let reads: Vec<ReadState> = all
            .iter()
            .map(|m| store.thread(m.thread).unwrap().summary.read)
            .collect();
        (patch.changes, reads)
    });
    let (changes, reads) = found;
    assert_eq!(
        changes,
        vec![Change::MessageRead(message(4).id, ReadState::Read)]
    );
    let expected: Vec<ReadState> = (1..=6)
        .map(|n| {
            if n == 4 {
                ReadState::Read
            } else {
                ReadState::Unread
            }
        })
        .collect();
    assert_eq!(reads, expected);
}

#[test]
fn a_cursor_that_has_not_moved_is_not_written_again_within_the_minute() {
    let store = sqlite();
    let batch = || Ingest {
        cursor: Some(SyncCursor::Pop),
        ..ingest("INBOX")
    };
    store.ingest(acct_account(), batch()).unwrap();
    let before = store.raw_connection().total_changes();

    store.ingest(acct_account(), batch()).unwrap();

    assert_eq!(store.raw_connection().total_changes(), before);
}
