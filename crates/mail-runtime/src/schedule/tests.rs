//! The scheduler's tables, and its loop with a host of the test's own: passes that end as
//! scripted, nothing that reaches a server.

use super::*;
use crate::fetch::{First, Pause, Step};
use chrono::TimeZone;
use mail_domain::Retry;
use mail_domain::SyncCursor;
use mail_domain::id::account_id_from_uuid;
use std::cell::RefCell;
use std::collections::BTreeSet;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e9"))
}

fn acct_new() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
fn acct_known() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}
fn acct_gone() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a3"))
}

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn current(live: Live) -> Link {
    Link::Current {
        at: at(0),
        trouble: vec![],
        live,
    }
}

fn waiting() -> Link {
    Link::Waiting {
        until: at(5),
        why: Pause::Unreachable,
        failures: 1,
        first: First::No,
    }
}

fn syncing_over(after: Link) -> Link {
    Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(after),
    }
}

fn finished() -> Event {
    Event::Finished { trouble: vec![] }
}

// --- timers -----------------------------------------------------------------------------

#[test]
fn a_wake_sleeps_until_its_time_and_a_moment_over() {
    const CASES: &[(i64, Duration)] = &[(0, SLACK), (-30, SLACK)];
    for (offset, expected) in CASES {
        assert_eq!(delay(at(*offset), at(0)), *expected, "{offset} s from now");
    }
    assert_eq!(delay(at(300), at(0)), Duration::from_secs(300) + SLACK);
}

#[test]
fn a_wake_at_the_end_of_time_is_capped_not_overflowed() {
    assert_eq!(delay(DateTime::<Utc>::MAX_UTC, at(0)), FAR + SLACK);
}

#[test]
fn a_newer_wait_makes_the_older_one_stale() {
    let generations = Generations::default();
    let first = generations.next(&acct_account());
    assert!(generations.current(&acct_account(), first));
    let second = generations.next(&acct_account());
    assert!(!generations.current(&acct_account(), first));
    assert!(generations.current(&acct_account(), second));
    generations.forget(&acct_account());
    assert!(!generations.current(&acct_account(), second));
}

// --- the live watch's tables ------------------------------------------------------------

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
fn a_wake_is_told_as_what_it_was() {
    assert_eq!(Heard::from(crate::Woke::Mail), Heard::Mail);
    assert_eq!(Heard::from(crate::Woke::Due), Heard::Due);
    assert_eq!(Heard::from(crate::Woke::Interval), Heard::Interval);
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
    assert_eq!(refused.wait, fetch::BACKOFF_CEILING);
    assert_eq!(refused.nudge, Some(Event::Start(Trigger::Poll)));
    let unsupported = reconnect(1, &Retry::Fatal("no push".to_owned()), floor);
    assert_eq!(unsupported.wait, fetch::BACKOFF_CEILING);
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
        (
            false,
            Push::Offered,
            Daemon::Absent,
            waiting(),
            Action::Start,
        ),
        (
            false,
            Push::Offered,
            Daemon::Absent,
            syncing_over(current(Live::Pushed)),
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
    let over = syncing_over(stuck("refused"));
    assert_eq!(
        decide(true, Push::Offered, Daemon::Absent, &over),
        Action::Stop
    );
}

#[test]
fn a_push_that_finds_a_pass_running_is_owed_a_pass_when_it_finishes() {
    let push = Event::Start(Trigger::Push);
    let syncing = || syncing_over(current(Live::Pushed));
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
            waiting(),
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

// --- who schedules ----------------------------------------------------------------------

/// What the scheduler does with each event, by who schedules and what the link is. Beside a
/// watch, the watch answers the timers and the server, and a person still asks; an account that
/// never fetched and a running pass are the window's even beside a watch; without a watch
/// everything runs.
#[test]
fn the_verdict_table() {
    use Verdict::{Defer, Drop, Run};
    type Row = (&'static str, Schedule, Vec<Link>, Vec<Event>, Verdict);
    let cases: Vec<Row> = vec![
        (
            "a watch defers the timers",
            Schedule::Watch,
            vec![current(Live::Polling), waiting()],
            vec![Event::Start(Trigger::Poll), Event::Tick],
            Defer,
        ),
        (
            "a watch drops the server's push",
            Schedule::Watch,
            vec![current(Live::Polling), waiting()],
            vec![Event::Start(Trigger::Push)],
            Drop,
        ),
        (
            "a person still asks beside a watch",
            Schedule::Watch,
            vec![current(Live::Polling), waiting()],
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
            vec![Link::Fresh, syncing_over(current(Live::Polling))],
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
            vec![
                current(Live::Polling),
                waiting(),
                Link::Fresh,
                syncing_over(current(Live::Polling)),
            ],
            vec![
                Event::Start(Trigger::Poll),
                Event::Start(Trigger::Push),
                Event::Tick,
                Event::Start(Trigger::Manual),
            ],
            Run,
        ),
    ];
    for (name, schedule, links, events, expected) in &cases {
        for link in links {
            for event in events {
                assert_eq!(
                    verdict(*schedule, link, event),
                    *expected,
                    "{name}: {event:?} on {link:?}"
                );
            }
        }
    }
}

// --- where a link begins ----------------------------------------------------------------

fn every() -> Duration {
    Duration::from_secs(300)
}

fn empty_store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (SqliteStore::in_memory(dir.path()).unwrap(), dir)
}

fn stamped(store: &SqliteStore, account: AccountId, path: &str, at: &str) {
    // The stamp belongs to an account the store has.
    mail_store::testing::seed_account(store, account.clone(), "x@example.test");
    mail_store::testing::seed_sync_state(store, account.clone(), path, &SyncCursor::Pop, Some(at));
}

#[test]
fn an_account_the_store_knows_nothing_of_begins_fresh() {
    let (store, _dir) = empty_store();
    assert_eq!(last_synced(&store, acct_new()), None);
    assert_eq!(probe(&store, acct_new(), at(0)), Link::Fresh);
    assert_eq!(Link::Fresh.first(), First::Yes);
}

#[test]
fn an_account_with_a_recorded_pass_begins_current_as_of_it() {
    let (store, _dir) = empty_store();
    stamped(&store, acct_known(), "INBOX", "2026-10-01 11:55:30");
    // The newest of its mailboxes' stamps, not the first.
    stamped(&store, acct_known(), "Sent", "2026-10-01 11:58:00");
    let stamp = Utc.with_ymd_and_hms(2026, 10, 1, 11, 58, 0).unwrap();
    assert_eq!(last_synced(&store, acct_known()), Some(stamp));
    assert_eq!(
        probe(&store, acct_known(), at(0)),
        Link::Current {
            at: stamp,
            trouble: vec![],
            live: Live::Polling
        }
    );
    // Another account's stamp is not this one's.
    assert_eq!(probe(&store, acct_new(), at(0)), Link::Fresh);
}

#[test]
fn the_table_of_what_a_link_begins_as() {
    let earlier = at(0) - TimeDelta::minutes(3);
    let later = at(0) + TimeDelta::minutes(3);
    let rests = |at| Link::Current {
        at,
        trouble: vec![],
        live: Live::Polling,
    };
    let cases: [(&str, Option<DateTime<Utc>>, bool, Link); 5] = [
        ("nothing at all", None, false, Link::Fresh),
        ("mail but no time", None, true, rests(at(0))),
        ("a time", Some(earlier), false, rests(earlier)),
        ("a time and mail", Some(earlier), true, rests(earlier)),
        // A clock that went backwards must not make the account "updated in the future".
        ("a time from the future", Some(later), true, rests(at(0))),
    ];
    for (name, last, mail, expected) in cases {
        assert_eq!(link_for(last, mail, at(0)), expected, "{name}");
    }
}

#[test]
fn each_account_it_is_given_gets_one_link() {
    // `initial` makes a link for each account it is given, and it is given the ones with a
    // server (`sync::due::intervals` leaves out the ones that keep mail here), so a local-only
    // account has none.
    let (store, _dir) = empty_store();
    let links = initial(
        &store,
        &[(acct_new(), every()), (acct_known(), every())],
        at(0),
    );
    assert_eq!(
        links.keys().cloned().collect::<Vec<_>>(),
        [acct_new(), acct_known()]
    );
}

/// What happened, the accounts that exist after it, and what the links do about it.
type Case = (&'static str, Vec<(AccountId, Duration)>, Changes);

#[test]
fn the_links_follow_the_accounts_as_they_come_and_go() {
    let known: BTreeSet<AccountId> = [acct_known(), acct_gone()].into();
    let cases: [Case; 4] = [
        (
            "nothing changed",
            vec![(acct_known(), every()), (acct_gone(), every())],
            Changes::default(),
        ),
        (
            "one added",
            vec![
                (acct_known(), every()),
                (acct_gone(), every()),
                (acct_new(), every()),
            ],
            Changes {
                added: vec![acct_new()],
                removed: vec![],
            },
        ),
        (
            "one removed",
            vec![(acct_known(), every())],
            Changes {
                added: vec![],
                removed: vec![acct_gone()],
            },
        ),
        (
            "swapped",
            vec![(acct_known(), every()), (acct_new(), every())],
            Changes {
                added: vec![acct_new()],
                removed: vec![acct_gone()],
            },
        ),
    ];
    for (name, wanted, expected) in cases {
        assert_eq!(reconcile(&known, &wanted), expected, "{name}");
    }
    // A changed interval is not a change of accounts.
    assert_eq!(
        reconcile(
            &known,
            &[
                (acct_known(), Duration::from_secs(60)),
                (acct_gone(), every())
            ]
        ),
        Changes::default()
    );
}

// --- the loop ---------------------------------------------------------------------------

/// A host whose passes end as the test says, and which writes down what it was asked and told.
struct Scripted {
    schedule: Schedule,
    /// How the next pass ends.
    ending: RefCell<Event>,
    passes: RefCell<Vec<AccountId>>,
    changes: RefCell<Vec<Change>>,
}

impl Scripted {
    fn ending(event: Event) -> Self {
        Scripted {
            schedule: Schedule::Window,
            ending: RefCell::new(event),
            passes: RefCell::new(Vec::new()),
            changes: RefCell::new(Vec::new()),
        }
    }

    fn runs(&self) -> usize {
        self.passes.borrow().len()
    }

    fn link(&self, account: &AccountId) -> Option<Link> {
        let changes = self.changes.borrow();
        changes.iter().rev().find_map(|change| match change {
            Change::Link(one, link) if one == account => Some(link.clone()),
            _ => None,
        })
    }
}

impl Host for Scripted {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn schedule(&self) -> Schedule {
        self.schedule
    }

    fn daemon(&self) -> Daemon {
        Daemon::Absent
    }

    fn pushers(&self) -> Vec<AccountId> {
        Vec::new()
    }

    fn folder_in_flight(&self, _: &AccountId) -> bool {
        false
    }

    fn timing(&self) -> Timing {
        // Nothing starts by itself: the test asks.
        Timing {
            beat: Duration::from_secs(3600),
            ..Timing::default()
        }
    }

    fn pass(&self, call: PassCall) -> Local<'_, Passed> {
        Box::pin(async move {
            self.passes.borrow_mut().push(call.account.clone());
            let _ = call.progress.send(Event::Stepped(Step::Flags, None));
            Passed {
                event: self.ending.borrow().clone(),
                stored: Stored::Maybe,
            }
        })
    }

    fn listen(&self, _: ListenCall) -> Local<'_, Result<(), Lost>> {
        Box::pin(async {
            Err(Lost {
                retry: Retry::Fatal("a test listened".to_owned()),
                why: "a test listened".to_owned(),
            })
        })
    }

    fn changed(&self, change: Change) {
        self.changes.borrow_mut().push(change);
    }
}

/// Poll `holds` until it does, or fail after a few seconds.
async fn eventually(what: &str, holds: impl Fn() -> bool) {
    for _ in 0..500 {
        if holds() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never saw: {what}");
}

fn one_account(link: Link) -> BTreeMap<AccountId, Link> {
    BTreeMap::from([(acct_account(), link)])
}

#[tokio::test]
async fn a_press_runs_one_pass_that_reports_and_ends() {
    let host = Scripted::ending(finished());
    let (_dir, store) = {
        let (store, dir) = empty_store();
        (dir, Arc::new(store))
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        &host,
        store,
        one_account(current(Live::Polling)),
        BTreeMap::from([(acct_account(), every())]),
        tx.clone(),
    );
    let driver = async {
        tx.send(Note::Event(acct_account(), Event::Start(Trigger::Manual)))
            .unwrap();
        eventually("a link settled current after a pass", || {
            host.runs() == 1
                && matches!(host.link(&acct_account()), Some(Link::Current { .. }))
                && host.changes.borrow().contains(&Change::Stored)
        })
        .await;
    };
    tokio::select! {
        _ = scheduler.run(rx, End::Never) => panic!("the scheduler ended on its own"),
        () = driver => {}
    }
    let seen = host.changes.borrow();
    let began = seen
        .iter()
        .position(|change| matches!(change, Change::Link(_, link) if link.is_busy()));
    assert!(began.is_some(), "the pass was never seen running: {seen:?}");
}

#[tokio::test]
async fn a_start_while_a_pass_runs_is_not_a_second_pass() {
    let host = Scripted::ending(finished());
    let (store, _dir) = empty_store();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        &host,
        Arc::new(store),
        one_account(current(Live::Polling)),
        BTreeMap::from([(acct_account(), every())]),
        tx.clone(),
    );
    let driver = async {
        // Both are in the inbox before the scheduler has run the first.
        for _ in 0..2 {
            tx.send(Note::Event(acct_account(), Event::Start(Trigger::Manual)))
                .unwrap();
        }
        eventually("the pass ended", || {
            matches!(host.link(&acct_account()), Some(Link::Current { .. }))
                && host.changes.borrow().contains(&Change::Stored)
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::select! {
        _ = scheduler.run(rx, End::Never) => panic!("the scheduler ended on its own"),
        () = driver => {}
    }
    assert_eq!(host.runs(), 1);
}

#[tokio::test]
async fn beside_a_watch_only_a_person_starts_a_pass() {
    let mut host = Scripted::ending(finished());
    host.schedule = Schedule::Watch;
    let (store, _dir) = empty_store();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        &host,
        Arc::new(store),
        one_account(current(Live::Polling)),
        BTreeMap::from([(acct_account(), every())]),
        tx.clone(),
    );
    let driver = async {
        tx.send(Note::Event(acct_account(), Event::Start(Trigger::Poll)))
            .unwrap();
        tx.send(Note::Event(acct_account(), Event::Start(Trigger::Push)))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(host.runs(), 0, "a second fetcher");
        tx.send(Note::Event(acct_account(), Event::Start(Trigger::Manual)))
            .unwrap();
        eventually("the person's pass", || host.runs() == 1).await;
    };
    tokio::select! {
        _ = scheduler.run(rx, End::Never) => panic!("the scheduler ended on its own"),
        () = driver => {}
    }
}

#[tokio::test]
async fn the_accounts_that_come_and_go_get_and_lose_their_link() {
    let host = Scripted::ending(finished());
    let (store, _dir) = empty_store();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        &host,
        Arc::new(store),
        one_account(current(Live::Polling)),
        BTreeMap::from([(acct_account(), every())]),
        tx.clone(),
    );
    let driver = async {
        tx.send(Note::Accounts(vec![(acct_new(), every())]))
            .unwrap();
        eventually("the new account fetched, the old one gone", || {
            host.changes
                .borrow()
                .contains(&Change::Gone(acct_account()))
                && matches!(host.link(&acct_new()), Some(Link::Current { .. }))
        })
        .await;
    };
    tokio::select! {
        _ = scheduler.run(rx, End::Never) => panic!("the scheduler ended on its own"),
        () = driver => {}
    }
    assert_eq!(*host.passes.borrow(), [acct_new()]);
}

#[tokio::test]
async fn a_refused_sign_in_ends_a_run_that_waits_for_that() {
    let refused = Event::Failed {
        retry: Retry::NeedsReauth,
        why: "the server refused the password".to_owned(),
        pause: Pause::ServerBusy,
    };
    let host = Scripted::ending(refused);
    let (store, _dir) = empty_store();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    // No account yet: the run starts from what it is told, as a watch does.
    let scheduler = Scheduler::new(
        &host,
        Arc::new(store),
        BTreeMap::new(),
        BTreeMap::new(),
        tx.clone(),
    );
    tx.send(Note::Accounts(vec![(acct_account(), every())]))
        .unwrap();
    let ends = tokio::time::timeout(Duration::from_secs(5), scheduler.run(rx, End::WhenStuck))
        .await
        .expect("the run never ended");
    assert_eq!(ends.len(), 1);
    assert!(
        matches!(ends[0].1, Link::NeedsSignIn { .. }),
        "{:?}",
        ends[0].1
    );
    assert_eq!(host.runs(), 1, "a refused sign-in is not tried again");
}

#[tokio::test]
async fn a_run_over_no_accounts_has_nothing_to_wait_for() {
    let host = Scripted::ending(finished());
    let (store, _dir) = empty_store();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let scheduler = Scheduler::new(
        &host,
        Arc::new(store),
        BTreeMap::new(),
        BTreeMap::new(),
        tx.clone(),
    );
    tx.send(Note::Accounts(Vec::new())).unwrap();
    let ends = tokio::time::timeout(Duration::from_secs(5), scheduler.run(rx, End::WhenStuck))
        .await
        .expect("the run never ended");
    assert!(ends.is_empty());
}
