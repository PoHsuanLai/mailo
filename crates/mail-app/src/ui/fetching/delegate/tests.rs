use super::super::tests::{account, acct_account, fetching, link, passer, settle};
use super::*;
use crate::ui::app::App;
use crate::ui::fixtures::{dispatching, rebuild_into};
use dioxus::dioxus_core::VirtualDom;
use mail_core::fetch::{Link, Trigger};
use mail_store::SqliteStore;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A window whose passes are the test's own and whose scheduling is `schedule`'s answer, which a
/// test can change while the window is open.
fn window_beside(
    store: Arc<SqliteStore>,
    schedule: Arc<AtomicBool>,
    passer: super::super::Passer,
) -> VirtualDom {
    dispatching();
    let delegate = Delegate(Arc::new(move || {
        if schedule.load(Ordering::SeqCst) {
            Schedule::Watch
        } else {
            Schedule::Window
        }
    }));
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(passer)
        .with_root_context(delegate);
    let _ = rebuild_into(&mut dom);
    dom
}

#[tokio::test]
async fn a_window_beside_a_watch_fetches_only_when_a_person_asks() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let watching = Arc::new(AtomicBool::new(true));
    let mut dom = window_beside(store, watching.clone(), passer);
    let fetching = fetching(&dom);

    // What a timer or the server would start is the watch's.
    for trigger in [Trigger::Poll, Trigger::Push] {
        fetching.sync_all(trigger);
    }
    fetching.send(acct_account(), Event::Tick);
    settle(&mut dom).await;
    assert_eq!(script.runs.load(Ordering::SeqCst), 0, "a second fetcher");
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "{:?}",
        link(&dom)
    );

    // Sync is a person's, and runs.
    fetching.sync_all(Trigger::Manual);
    settle(&mut dom).await;
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
    script
        .release
        .lock()
        .unwrap()
        .send(super::super::tests::finished)
        .unwrap();
    settle(&mut dom).await;
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "{:?}",
        link(&dom)
    );

    // With no watch the same requests are the window's again.
    watching.store(false, Ordering::SeqCst);
    fetching.sync_all(Trigger::Poll);
    settle(&mut dom).await;
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        2,
        "the window did not take over"
    );
}

#[tokio::test]
async fn a_new_account_is_fetched_by_the_window_even_beside_a_watch() {
    let (store, _dir) = account(false);
    let (passer, script) = passer();
    let mut dom = window_beside(store, Arc::new(AtomicBool::new(true)), passer);
    assert_eq!(link(&dom), Link::Fresh);
    fetching(&dom).sync_all(Trigger::Poll);
    // A slow runner can start the fetch after one quiet stretch: wait for the run, not for quiet.
    for _ in 0..50 {
        settle(&mut dom).await;
        if script.runs.load(Ordering::SeqCst) > 0 {
            break;
        }
    }
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
}
