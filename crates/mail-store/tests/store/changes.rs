//! What the store says when it commits (`mail_store::changes`): what a write touched, once per
//! hold of the writer, nothing for a write that did not commit, and a read that is one moment of
//! the database however many commits land while it runs.

use crate::support::*;
use mail_domain::*;
use mail_store::testing::{hold_writer, in_memory, seed_account};
use mail_store::{Committed, SqliteStore, Store, Topics};
use std::sync::mpsc::Receiver;

/// A view that lists unread mail, called `name`.
fn view(name: &str) -> View {
    View {
        id: ViewId::generate(),
        name: name.to_owned(),
        kind: ViewKind::Query,
        filter: Filter::Read(ReadState::Unread),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        group_by: None,
        threading: Threading::Threaded,
        hover: Vec::new(),
    }
}

/// Everything said so far.
fn heard(rx: &Receiver<Committed>) -> Vec<Committed> {
    rx.try_iter().collect()
}

#[test]
fn a_write_says_what_it_touched_and_counts_one_more_generation() {
    let store = in_memory();
    let rx = store.subscribe();
    let before = store.generation();

    store.put_view(&view("Unread")).unwrap();

    let said = heard(&rx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].topics, Topics::VIEWS);
    assert_eq!(said[0].generation.0, before.0 + 1);
    assert_eq!(store.generation(), said[0].generation);
}

#[test]
fn a_patch_says_what_its_changes_touch_and_no_more() {
    let store = sqlite();
    let m = message(1);
    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageUpsert(Box::new(m.clone()))]),
        )
        .unwrap();
    let rx = store.subscribe();

    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageStar(m.id, Star::Starred)]),
        )
        .unwrap();

    let said = heard(&rx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].topics, Topics::SUMMARIES);
}

#[test]
fn a_commit_that_said_nothing_says_everything() {
    let store = in_memory();
    let rx = store.subscribe();

    seed_account(&store, acct_account(), "me@example.test");

    let said = heard(&rx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].topics, Topics::ALL);
}

#[test]
fn a_transaction_rolled_back_says_nothing() {
    let store = in_memory();
    let rx = store.subscribe();
    let before = store.generation();
    {
        let db = store.raw_connection();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES ('x', 'x@example.test', '{}', datetime('now'))",
            [],
        )
        .unwrap();
        tx.rollback().unwrap();
    }
    // Nor does a write that matched no row: it committed, and changed nothing.
    let refused = store.delete_view(ViewId::generate());
    assert!(refused.is_err());

    assert!(heard(&rx).is_empty());
    assert_eq!(store.generation(), before);
}

#[test]
fn writes_under_one_hold_of_the_writer_are_said_once_when_it_is_let_go_of() {
    let store = sqlite();
    let rx = store.subscribe();
    {
        let _held = hold_writer(&store);
        store.put_view(&view("One")).unwrap();
        store.put_view(&view("Two")).unwrap();
        store.create_label(acct_account(), "Travel").unwrap();
        assert!(
            heard(&rx).is_empty(),
            "said before the writer was let go of"
        );
    }
    let said = heard(&rx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].topics, Topics::VIEWS | Topics::LABELS);
}

#[test]
fn removing_an_account_says_everything() {
    let store = sqlite();
    let rx = store.subscribe();

    store.remove_account(acct_other()).unwrap();

    let said = heard(&rx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].topics, Topics::ALL);
}

#[test]
fn a_subscriber_that_has_gone_does_not_stop_the_others_hearing() {
    let store = in_memory();
    let gone = store.subscribe();
    let kept = store.subscribe();
    drop(gone);

    store.put_view(&view("Unread")).unwrap();
    store.put_view(&view("Again")).unwrap();

    assert_eq!(heard(&kept).len(), 2);
}

/// A read pinned on one thread sees the database as it was when it began, while another thread
/// commits in the middle of it; the next read sees the commit.
#[test]
fn a_pinned_read_is_one_moment_of_the_database_across_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    store.put_view(&view("Before")).unwrap();

    let (first, second) = store
        .read_pinned(|| {
            let first = store.views().unwrap().len();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| store.put_view(&view("During")).unwrap())
                    .join()
                    .unwrap();
            });
            let second = store.views().unwrap().len();
            (first, second)
        })
        .unwrap();

    assert_eq!((first, second), (1, 1), "the commit landed inside the read");
    assert_eq!(store.views().unwrap().len(), 2, "and is there once it ends");
}

/// Unpinned, the same reads straddle the commit: what the pin is for.
#[test]
fn an_unpinned_read_sees_a_commit_made_between_two_of_its_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    store.put_view(&view("Before")).unwrap();

    let first = store.views().unwrap().len();
    std::thread::scope(|scope| {
        scope
            .spawn(|| store.put_view(&view("During")).unwrap())
            .join()
            .unwrap();
    });
    let second = store.views().unwrap().len();

    assert_eq!((first, second), (1, 2));
}

#[test]
fn a_read_pinned_inside_another_runs_in_the_same_moment() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();

    let inner = store
        .read_pinned(|| {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| store.put_view(&view("During")).unwrap())
                    .join()
                    .unwrap();
            });
            store.read_pinned(|| store.views().unwrap().len()).unwrap()
        })
        .unwrap();

    assert_eq!(inner, 0);
}

#[test]
fn a_wake_that_subscribes_does_not_deadlock_the_commit_that_called_it() {
    // The wake runs on the committing thread: one that takes the subscribers' own lock, as a
    // window opening a second listener would, must not be called with that lock held.
    let store = std::sync::Arc::new(in_memory());
    let later: std::sync::Arc<std::sync::Mutex<Vec<Receiver<Committed>>>> = Default::default();
    let (inside, kept) = (store.clone(), later.clone());
    let _first = store.subscribe_waking(std::sync::Arc::new(move || {
        kept.lock().unwrap().push(inside.subscribe());
    }));

    let (done, finished) = std::sync::mpsc::channel();
    let writing = store.clone();
    std::thread::spawn(move || {
        writing.put_view(&view("Unread")).unwrap();
        done.send(()).unwrap();
    });

    assert!(
        finished
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok(),
        "the commit waited on its own subscribers"
    );
    assert_eq!(later.lock().unwrap().len(), 1);
}
