use super::super::tests::{account, acct_account, fetching, link, passer, settle};
use super::*;
use crate::ui::app::App;
use crate::ui::fixtures::{dispatching, rebuild_into};
use chrono::{TimeZone, Utc};
use dioxus::dioxus_core::VirtualDom;
use mail_core::SqliteStore;
use mail_core::fetch::{First, Live, Pause, Step};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn current() -> Link {
    Link::Current {
        at: Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap(),
        trouble: vec![],
        live: Live::Polling,
    }
}

fn waiting() -> Link {
    Link::Waiting {
        until: Utc.with_ymd_and_hms(2026, 10, 1, 12, 5, 0).unwrap(),
        why: Pause::Unreachable,
        failures: 1,
        first: First::No,
    }
}

fn syncing() -> Link {
    Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(current()),
    }
}

fn fresh() -> Link {
    Link::Fresh
}

/// What the runner does with each event, by who schedules and what the link is. Beside a watch,
/// the watch answers the timers and the server, and a person still asks; an account that never
/// fetched and a running pass are the window's even beside a watch; without a watch everything
/// runs.
#[test]
fn the_verdict_table() {
    use Verdict::{Defer, Drop, Run};
    type Row = (
        &'static str,
        Schedule,
        &'static [fn() -> Link],
        Vec<Event>,
        Verdict,
    );
    let cases: &[Row] = &[
        (
            "a watch defers the timers",
            Schedule::Watch,
            &[current, waiting],
            vec![Event::Start(Trigger::Poll), Event::Tick],
            Defer,
        ),
        (
            "a watch drops the server's push",
            Schedule::Watch,
            &[current, waiting],
            vec![Event::Start(Trigger::Push)],
            Drop,
        ),
        (
            "a person still asks beside a watch",
            Schedule::Watch,
            &[current, waiting],
            vec![
                Event::Start(Trigger::Manual),
                Event::Start(Trigger::FolderOpen),
                Event::SignedIn,
                Event::Cancel,
                Event::Live(Live::Pushed),
            ],
            Run,
        ),
        (
            "never fetched or mid-pass is the window's beside a watch",
            Schedule::Watch,
            &[fresh, syncing],
            vec![
                Event::Start(Trigger::Poll),
                Event::Start(Trigger::Push),
                Event::Tick,
            ],
            Run,
        ),
        (
            "without a watch everything runs",
            Schedule::Window,
            &[current, waiting, fresh, syncing],
            vec![
                Event::Start(Trigger::Poll),
                Event::Start(Trigger::Push),
                Event::Tick,
                Event::Start(Trigger::Manual),
            ],
            Run,
        ),
    ];
    for (name, schedule, links, events, expected) in cases {
        for link in links.iter().map(|make| make()) {
            for event in events {
                assert_eq!(
                    verdict(*schedule, &link, event),
                    *expected,
                    "{name}: {event:?} on {link:?}"
                );
            }
        }
    }
}

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
