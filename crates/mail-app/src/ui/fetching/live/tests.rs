//! The live watch in the running window, with a watch of the test's own and passes held shut
//! until it lets them go, as `fetching/tests.rs` does for the passes alone.

use super::*;
use crate::ui::fetching::{Fetching, Passer};
use crate::ui::fixtures::{dispatching, empty};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_core::SqliteStore;
use mail_core::Store as _;
use mail_core::fetch::{Link, Live, Pause, Trigger};
use mail_core::sync::report::{AccountReport, Counts, PassEnd};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e9"))
}

/// Whether a watch has been told to stop, or its owner is gone.
fn stopped(stop: &Stop) -> bool {
    stop.has_changed().map_or(true, |_| *stop.borrow())
}

// --- the window -------------------------------------------------------------------------

/// A store with one IMAP account that has been fetched before.
fn account() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let (store, dir) = empty();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@nowhere.example", &manual, Utc::now());
    mail_store::testing::seed_account_plan(
        &store,
        acct_account(),
        &preset.plan.address,
        &preset.plan,
        Some(Utc::now()),
    );
    mail_store::testing::seed_sync_state(&store, acct_account(), "INBOX", &SyncCursor::Pop, None);
    store
        .put_caps(acct_account(), &preset.expected_caps, Utc::now())
        .unwrap();
    (store, dir)
}

type Ending = fn() -> PassEnd;

fn ended() -> PassEnd {
    PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        counts: Counts::default(),
        trouble: vec![],
    })
}

fn refused() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::NeedsReauth,
        why: "the server refused the password".to_owned(),
        pause: Pause::ServerBusy,
    }
}

fn unreachable() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::After(Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: Pause::Unreachable,
    }
}

/// What the test's watch does next.
enum Then {
    Hear(Heard),
    Lose(Retry),
}

/// The test's passes and watch, and what it can see of them.
#[derive(Clone, Default)]
struct Script {
    runs: Arc<AtomicUsize>,
    /// Watches begun, and watches running now.
    starts: Arc<AtomicUsize>,
    alive: Arc<AtomicUsize>,
    then: Arc<Mutex<VecDeque<Then>>>,
    daemon: Arc<AtomicBool>,
    release: Arc<Mutex<Option<mpsc::Sender<Ending>>>>,
}

impl Script {
    fn passer(&self) -> Passer {
        let (release, waiting) = mpsc::channel::<Ending>();
        *self.release.lock().unwrap() = Some(release);
        let waiting = Arc::new(Mutex::new(waiting));
        let runs = self.runs.clone();
        Passer(Arc::new(move |_store, _now, _account, _hooks| {
            runs.fetch_add(1, Ordering::SeqCst);
            let end = waiting
                .lock()
                .unwrap()
                .recv()
                .map_err(|_| "never let go".to_owned())?;
            Ok(vec![end()])
        }))
    }

    fn listener(&self) -> Listener {
        let (starts, alive, then) = (self.starts.clone(), self.alive.clone(), self.then.clone());
        let daemon = self.daemon.clone();
        Listener {
            listen: Arc::new(move |_store, _account, stop, _hold, heard| {
                starts.fetch_add(1, Ordering::SeqCst);
                alive.fetch_add(1, Ordering::SeqCst);
                loop {
                    if stopped(&stop) {
                        alive.fetch_sub(1, Ordering::SeqCst);
                        return Ok(());
                    }
                    match then.lock().unwrap().pop_front() {
                        Some(Then::Hear(said)) => heard(said),
                        Some(Then::Lose(retry)) => {
                            alive.fetch_sub(1, Ordering::SeqCst);
                            return Err(Lost {
                                retry,
                                why: "the connection dropped".to_owned(),
                            });
                        }
                        None => {}
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }),
            pushers: Arc::new(|_| vec![acct_account()]),
            daemon: Arc::new(move || {
                if daemon.load(Ordering::SeqCst) {
                    Daemon::Holding
                } else {
                    Daemon::Absent
                }
            }),
            first: Duration::ZERO,
            retry: Duration::from_millis(20),
        }
    }

    fn say(&self, then: Then) {
        self.then.lock().unwrap().push_back(then);
    }

    fn let_go(&self, end: Ending) {
        self.release
            .lock()
            .unwrap()
            .as_ref()
            .expect("a passer was made")
            .send(end)
            .unwrap();
    }

    fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }

    fn alive(&self) -> usize {
        self.alive.load(Ordering::SeqCst)
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

/// Just the fetching, with the revision the window would bump in the context.
#[component]
fn Probe() -> Element {
    let revision = use_signal(|| 0u64);
    use_context_provider(|| revision);
    super::super::use_fetching(revision);
    rsx! {}
}

fn window(store: Arc<SqliteStore>, script: &Script) -> VirtualDom {
    dispatching();
    let mut dom = VirtualDom::new(Probe)
        .with_root_context(store)
        .with_root_context(script.passer())
        .with_root_context(script.listener());
    dom.rebuild_in_place();
    dom
}

/// Let what is waiting land and redraw, as the window would between frames.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..12 {
        if tokio::time::timeout(Duration::from_millis(80), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut NoOpMutations);
    }
}

/// Settle until `holds`, for up to two seconds: a watch and its reconnects run on threads and
/// timers the window does not wait for.
async fn eventually(dom: &mut VirtualDom, what: &str, holds: impl Fn(&VirtualDom) -> bool) {
    for _ in 0..40 {
        settle(dom).await;
        if holds(dom) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}

fn fetching(dom: &VirtualDom) -> Fetching {
    dom.in_scope(ScopeId::APP, consume_context::<Fetching>)
}

fn link(dom: &VirtualDom) -> Link {
    dom.in_scope(ScopeId::APP, || fetching(dom).link(acct_account()))
        .expect("the account has a link")
}

fn revision(dom: &VirtualDom) -> u64 {
    dom.in_scope(ScopeId::APP, || *consume_context::<Signal<u64>>().peek())
}

fn is_live(dom: &VirtualDom) -> bool {
    matches!(
        link(dom),
        Link::Current {
            live: Live::Pushed,
            ..
        }
    )
}

fn is_polling(dom: &VirtualDom) -> bool {
    matches!(
        link(dom),
        Link::Current {
            live: Live::Polling,
            ..
        }
    )
}

#[tokio::test]
async fn an_established_watch_makes_the_account_live_and_a_lost_one_puts_it_back_on_the_timer() {
    let (store, _dir) = account();
    let script = Script::default();
    let mut dom = window(store, &script);
    eventually(&mut dom, "the watch began", |_| script.starts() == 1).await;
    assert!(is_polling(&dom), "not live until the watch says so");

    script.say(Then::Hear(Heard::Established));
    eventually(&mut dom, "live", is_live).await;

    // Lost: back to polling at once, then a new connection after the backoff, and live again.
    script.say(Then::Lose(Retry::Now));
    script.say(Then::Hear(Heard::Established));
    eventually(&mut dom, "reconnected", |_| script.starts() == 2).await;
    eventually(&mut dom, "live again", is_live).await;
    assert_eq!(script.runs(), 0, "a watch is not a pass");
}

#[tokio::test]
async fn mail_heard_starts_a_pass_and_one_heard_during_it_starts_another_after() {
    let (store, _dir) = account();
    let script = Script::default();
    let mut dom = window(store, &script);
    eventually(&mut dom, "the watch began", |_| script.starts() == 1).await;
    script.say(Then::Hear(Heard::Established));
    eventually(&mut dom, "live", is_live).await;

    script.say(Then::Hear(Heard::Mail));
    eventually(&mut dom, "a pass for the push", |_| script.runs() == 1).await;
    assert!(link(&dom).is_busy());

    // More mail while it runs: the link ignores the start, and it is not lost.
    script.say(Then::Hear(Heard::Mail));
    settle(&mut dom).await;
    assert_eq!(script.runs(), 1, "two passes on one account");

    script.let_go(ended);
    eventually(&mut dom, "the owed pass", |_| script.runs() == 2).await;
    script.let_go(ended);
    eventually(&mut dom, "current again", |dom| !link(dom).is_busy()).await;
    assert_eq!(script.runs(), 2, "owed once, not for ever");
    assert!(is_live(&dom), "the watch's word survived the passes");
}

#[tokio::test]
async fn the_watch_stops_when_the_sign_in_is_refused_and_resumes_when_it_is_replaced() {
    let (store, _dir) = account();
    let script = Script::default();
    let mut dom = window(store, &script);
    eventually(&mut dom, "the watch began", |_| script.alive() == 1).await;

    fetching(&dom).sync_all(Trigger::Manual);
    eventually(&mut dom, "a pass", |_| script.runs() == 1).await;
    script.let_go(refused);
    eventually(&mut dom, "needs sign-in", |dom| {
        matches!(link(dom), Link::NeedsSignIn { .. })
    })
    .await;
    eventually(&mut dom, "the watch stopped", |_| script.alive() == 0).await;
    let begun = script.starts();

    fetching(&dom).signed_in(acct_account());
    eventually(&mut dom, "a pass", |_| script.runs() == 2).await;
    script.let_go(ended);
    eventually(&mut dom, "the watch resumed", |_| {
        script.alive() == 1 && script.starts() == begun + 1
    })
    .await;
    assert!(matches!(link(&dom), Link::Current { .. }));
}

#[tokio::test]
async fn a_daemon_holding_the_account_means_the_window_does_not_watch() {
    let (store, _dir) = account();
    let script = Script::default();
    script.daemon.store(true, Ordering::SeqCst);
    let mut dom = window(store, &script);
    settle(&mut dom).await;
    fetching(&dom).sync_all(Trigger::Manual);
    eventually(&mut dom, "a pass", |_| script.runs() == 1).await;
    script.let_go(ended);
    eventually(&mut dom, "current", is_polling).await;
    assert_eq!(script.starts(), 0, "two watchers on one account");
    assert!(is_polling(&dom), "it polls like an account with no push");
}

#[tokio::test]
async fn a_pass_that_could_not_run_does_not_redraw_the_window() {
    let (store, _dir) = account();
    let script = Script::default();
    let mut dom = window(store, &script);
    settle(&mut dom).await;
    let before = revision(&dom);

    fetching(&dom).sync_all(Trigger::Manual);
    eventually(&mut dom, "a pass", |_| script.runs() == 1).await;
    script.let_go(unreachable);
    eventually(&mut dom, "waiting", |dom| {
        matches!(link(dom), Link::Waiting { .. })
    })
    .await;
    assert_eq!(
        revision(&dom),
        before,
        "an empty failed pass was read again"
    );

    fetching(&dom).sync_all(Trigger::Manual);
    eventually(&mut dom, "a pass", |_| script.runs() == 2).await;
    script.let_go(ended);
    eventually(&mut dom, "current", |dom| !link(dom).is_busy()).await;
    assert_eq!(revision(&dom), before + 1, "a pass that ran is read");
}
