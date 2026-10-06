//! The live watch in the running window, with a watch of the test's own and passes held shut
//! until it lets them go, as `fetching/tests.rs` does for the passes alone.

use super::{Action, Event, *};
use crate::ui::fetching::{Fetching, Passer};
use crate::ui::fixtures::{dispatching, empty};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_core::fetch::{First, Pause, Step};
use mail_core::sync::report::{AccountReport, Counts, PassEnd};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::Store as _;
use porter_core::AccountId;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, mpsc};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e9"))
}

fn at(second: i64) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + chrono::TimeDelta::seconds(second)
}

fn current(live: Live) -> Link {
    Link::Current {
        at: at(0),
        trouble: vec![],
        live,
    }
}

fn syncing() -> Link {
    Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(current(Live::Pushed)),
    }
}

fn finished() -> Event {
    Event::Finished { trouble: vec![] }
}

// --- the tables -------------------------------------------------------------------------

#[test]
fn what_a_watch_hears_is_what_the_link_is_told() {
    let table = [
        (Heard::Established, Event::Live(Live::Pushed)),
        (Heard::Mail, Event::Start(Trigger::Push)),
        (Heard::Due, Event::Start(Trigger::Poll)),
        (Heard::Interval, Event::Start(Trigger::Poll)),
    ];
    for (heard, event) in table {
        assert_eq!(event_for(heard), event, "{heard:?}");
    }
}

#[test]
fn reconnecting_backs_off_and_never_spins() {
    let floor = Duration::from_secs(5);
    let waits: Vec<u64> = (1..=12)
        .map(|n| reconnect(n, &Retry::Now, floor).wait.as_secs())
        .collect();
    assert_eq!(
        waits,
        [5, 10, 20, 40, 80, 160, 320, 640, 1280, 1800, 1800, 1800]
    );
    assert!(waits.iter().all(|wait| *wait >= floor.as_secs()));
    // The server's own ask is a floor under the schedule, not a replacement for it.
    let asked = Retry::After(Duration::from_secs(90));
    assert_eq!(reconnect(1, &asked, floor).wait, Duration::from_secs(90));
    // At the fifth loss the schedule says 80 s, and the server's 90 s is longer.
    assert_eq!(reconnect(5, &asked, floor).wait, Duration::from_secs(90));
}

#[test]
fn a_refused_sign_in_is_never_retried_in_a_loop() {
    let floor = Duration::from_secs(5);
    let refused = reconnect(1, &Retry::NeedsReauth, floor);
    assert_eq!(refused.wait, BACKOFF_CEILING);
    assert_eq!(refused.nudge, Some(Event::Start(Trigger::Poll)));
    let unsupported = reconnect(1, &Retry::Fatal("no push".to_owned()), floor);
    assert_eq!(unsupported.wait, BACKOFF_CEILING);
    assert_eq!(unsupported.nudge, None);
}

#[test]
fn a_watch_runs_when_pushed_to_unclaimed_and_not_stopped_for_a_person() {
    let stuck = |why: &str| Link::NeedsSignIn {
        why: why.to_owned(),
        first: First::No,
    };
    let broken = Link::Broken {
        why: "x".to_owned(),
        first: First::No,
    };
    let waiting = Link::Waiting {
        until: at(5),
        why: Pause::Unreachable,
        failures: 1,
        first: First::No,
    };
    let table = [
        // (running, push, daemon, link, action)
        (
            false,
            Push::Offered,
            Daemon::Absent,
            current(Live::Polling),
            Action::Start,
        ),
        (
            false,
            Push::Offered,
            Daemon::Absent,
            Link::Fresh,
            Action::Start,
        ),
        (false, Push::Offered, Daemon::Absent, waiting, Action::Start),
        (
            false,
            Push::Offered,
            Daemon::Absent,
            syncing(),
            Action::Start,
        ),
        (
            true,
            Push::Offered,
            Daemon::Absent,
            current(Live::Pushed),
            Action::Keep,
        ),
        // Nobody to wait on, or somebody else waiting.
        (
            false,
            Push::Absent,
            Daemon::Absent,
            current(Live::Polling),
            Action::Keep,
        ),
        (
            true,
            Push::Absent,
            Daemon::Absent,
            current(Live::Pushed),
            Action::Stop,
        ),
        (
            false,
            Push::Offered,
            Daemon::Holding,
            current(Live::Polling),
            Action::Keep,
        ),
        (
            true,
            Push::Offered,
            Daemon::Holding,
            current(Live::Pushed),
            Action::Stop,
        ),
        // Only a person clears these.
        (
            true,
            Push::Offered,
            Daemon::Absent,
            stuck("refused"),
            Action::Stop,
        ),
        (
            false,
            Push::Offered,
            Daemon::Absent,
            stuck("refused"),
            Action::Keep,
        ),
        (true, Push::Offered, Daemon::Absent, broken, Action::Stop),
    ];
    for (running, push, daemon, link, action) in table {
        assert_eq!(
            decide(running, push, daemon, &link),
            action,
            "{running} {push:?} {daemon:?} {link:?}"
        );
    }
    // A pass running over a refused account is still a refused account.
    let over = Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(stuck("refused")),
    };
    assert_eq!(
        decide(true, Push::Offered, Daemon::Absent, &over),
        Action::Stop
    );
}

#[test]
fn a_push_that_finds_a_pass_running_is_owed_a_pass_when_it_finishes() {
    let push = Event::Start(Trigger::Push);
    let mut dirty = Dirty::default();

    // Heard while idle: the link starts a pass for it, nothing is owed.
    dirty.heard(acct_account(), &current(Live::Pushed), &push);
    assert_eq!(
        dirty.settled(
            acct_account(),
            &syncing(),
            &current(Live::Pushed),
            &finished()
        ),
        Rerun::No
    );

    // Heard while syncing: owed once, and only to a pass that finished.
    dirty.heard(acct_account(), &syncing(), &Event::Start(Trigger::Poll));
    dirty.heard(acct_account(), &syncing(), &push);
    assert_eq!(
        dirty.settled(acct_account(), &syncing(), &syncing(), &Event::Tick),
        Rerun::No,
        "still running"
    );
    assert_eq!(
        dirty.settled(
            acct_account(),
            &syncing(),
            &current(Live::Pushed),
            &finished()
        ),
        Rerun::Yes
    );
    assert_eq!(
        dirty.settled(
            acct_account(),
            &syncing(),
            &current(Live::Pushed),
            &finished()
        ),
        Rerun::No,
        "owed once"
    );

    // A pass that failed has its own backoff; one that was cancelled is not restarted.
    for (after, event) in [
        (
            Link::Waiting {
                until: at(5),
                why: Pause::Unreachable,
                failures: 1,
                first: First::No,
            },
            Event::Failed {
                retry: Retry::Now,
                why: "x".to_owned(),
                pause: Pause::Unreachable,
            },
        ),
        (current(Live::Pushed), Event::Cancel),
    ] {
        dirty.heard(acct_account(), &syncing(), &push);
        assert_eq!(
            dirty.settled(acct_account(), &syncing(), &after, &event),
            Rerun::No
        );
        assert_eq!(
            dirty.settled(
                acct_account(),
                &syncing(),
                &current(Live::Pushed),
                &finished()
            ),
            Rerun::No,
            "the debt was cleared with the pass that ended"
        );
    }
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
    let db = store.connection();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
        [
            acct_account().to_string(),
            preset.plan.address.clone(),
            serde_json::to_string(&preset.plan).unwrap(),
            Utc::now().to_rfc3339(),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sync_state (account, mailbox, cursor, synced_at)
         VALUES (?1, 'INBOX', ?2, datetime('now'))",
        rusqlite::params![
            acct_account().to_string(),
            serde_json::to_string(&SyncCursor::Pop).unwrap()
        ],
    )
    .unwrap();
    drop(db);
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
