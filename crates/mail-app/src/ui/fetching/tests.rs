//! The effect runner in the running window: a pass is asked for, runs off the thread that draws,
//! reports where it is, and ends; the link in between is what the list reads. The passes here
//! are the test's own, held shut until it lets them go, so that "running" can be looked at.

use super::{Fetching, Passer};
use crate::ui::app::App;
use crate::ui::fixtures::{Seen, click, dispatching, empty, rebuild_into};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use ds::motion::detail::operation::Operation;
use mail_core::fetch::{Count, Event, First, Link, Step, Trigger};
use mail_core::sync::report::{AccountReport, Counts, PassEnd, Progress};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

pub(super) fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c9"))
}

/// A store with one IMAP account. With `fetched`, an earlier pass is on record for it.
pub(super) fn account(fetched: bool) -> (Arc<SqliteStore>, tempfile::TempDir) {
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
    if fetched {
        db.execute(
            "INSERT INTO sync_state (account, mailbox, cursor, synced_at)
             VALUES (?1, 'INBOX', ?2, datetime('now'))",
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&SyncCursor::Pop).unwrap()
            ],
        )
        .unwrap();
    }
    drop(db);
    store
        .put_caps(acct_account(), &preset.expected_caps, Utc::now())
        .unwrap();
    (store, dir)
}

/// How a held pass ends once it is let go.
pub(super) type Ending = fn() -> PassEnd;

/// What a test's pass does when it is run: say where it is, wait to be let go, then end as told.
#[derive(Clone)]
pub(super) struct Script {
    pub(super) runs: Arc<AtomicUsize>,
    /// Lets the next waiting pass finish with this end.
    pub(super) release: Arc<Mutex<mpsc::Sender<Ending>>>,
}

pub(super) fn finished() -> PassEnd {
    PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        counts: Counts::default(),
        trouble: vec![],
    })
}

pub(super) fn refused() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::NeedsReauth,
        why: "the server refused the password".to_owned(),
        pause: mail_core::fetch::Pause::ServerBusy,
    }
}

pub(super) fn unreachable() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::After(Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: mail_core::fetch::Pause::Unreachable,
    }
}

pub(super) fn passer() -> (Passer, Script) {
    let runs = Arc::new(AtomicUsize::new(0));
    let (release, waiting) = mpsc::channel::<Ending>();
    let waiting = Arc::new(Mutex::new(waiting));
    let counted = runs.clone();
    let passer = Passer(Arc::new(move |_store, _now, account, hooks| {
        counted.fetch_add(1, Ordering::SeqCst);
        let say = hooks
            .progress
            .expect("a pass is given somewhere to say where it is");
        say(
            account,
            Progress::Headers {
                mailbox: "INBOX".to_owned(),
                done: 3,
                of: Some(10),
            },
        );
        let end = waiting
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| "never let go".to_owned())?;
        Ok(vec![end()])
    }));
    let script = Script {
        runs,
        release: Arc::new(Mutex::new(release)),
    };
    (passer, script)
}

pub(super) fn window(store: Arc<SqliteStore>, passer: Passer) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(passer);
    let seen = rebuild_into(&mut dom);
    (dom, seen)
}

/// Let what is waiting land and redraw, as the window would between frames.
pub(super) async fn settle(dom: &mut VirtualDom) {
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

pub(super) fn fetching(dom: &VirtualDom) -> Fetching {
    dom.in_scope(ScopeId::APP, consume_context::<Fetching>)
}

pub(super) fn link(dom: &VirtualDom) -> Link {
    dom.in_scope(ScopeId::APP, || fetching(dom).link(acct_account()))
        .expect("the account has a link")
}

fn is_running(dom: &VirtualDom) -> bool {
    dom.in_scope(ScopeId::APP, || {
        matches!(fetching(dom).op(acct_account()), Operation::Running(_))
    })
}

#[tokio::test]
async fn pressing_sync_runs_one_pass_that_reports_and_ends() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, seen) = window(store, passer);
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "{:?}",
        link(&dom)
    );
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        0,
        "nothing ran before it was asked"
    );

    click(&mut dom, seen.one("aria-label", "Sync now"));
    settle(&mut dom).await;
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
    match link(&dom) {
        Link::Syncing {
            first: First::No,
            step: Step::Headers { mailbox },
            count: Some(Count { done: 3, of: 10 }),
            ..
        } => assert_eq!(mailbox, "INBOX"),
        other => panic!("the pass's progress did not reach the link: {other:?}"),
    }
    assert!(is_running(&dom), "no spinner while a pass runs");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Downloading 3 of 10"),
        "the list does not say what the pass says: {page}"
    );

    // A second press is the same request: nothing new starts.
    click(&mut dom, seen.one("aria-label", "Sync now"));
    settle(&mut dom).await;
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        1,
        "two passes on one account"
    );

    script.release.lock().unwrap().send(finished).unwrap();
    settle(&mut dom).await;
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "{:?}",
        link(&dom)
    );
    assert!(!is_running(&dom), "the spinner outlived the pass");
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_refused_sign_in_stops_that_account_until_it_is_signed_in_again() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, _seen) = window(store, passer);
    let fetching = fetching(&dom);
    fetching.sync_all(Trigger::Manual);
    settle(&mut dom).await;
    script.release.lock().unwrap().send(refused).unwrap();
    settle(&mut dom).await;
    assert!(
        matches!(link(&dom), Link::NeedsSignIn { .. }),
        "{:?}",
        link(&dom)
    );
    assert!(!is_running(&dom));

    // Neither a timer nor a person pressing Sync knocks on a server that has said no.
    for trigger in [
        Trigger::Poll,
        Trigger::Push,
        Trigger::Manual,
        Trigger::FolderOpen,
    ] {
        fetching.sync_all(trigger);
    }
    fetching.send(acct_account(), Event::Tick);
    settle(&mut dom).await;
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        1,
        "it tried a refused credential again"
    );

    fetching.signed_in(acct_account());
    settle(&mut dom).await;
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        2,
        "signing in did not start a pass"
    );
    script.release.lock().unwrap().send(finished).unwrap();
    settle(&mut dom).await;
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "{:?}",
        link(&dom)
    );
}

#[tokio::test]
async fn a_first_pass_that_fails_is_still_the_first_when_it_is_tried_again() {
    let (store, _dir) = account(false);
    let (passer, script) = passer();
    let (mut dom, _seen) = window(store, passer);
    assert_eq!(link(&dom), Link::Fresh);
    let fetching = fetching(&dom);
    fetching.sync_all(Trigger::Manual);
    settle(&mut dom).await;
    assert!(matches!(
        link(&dom),
        Link::Syncing {
            first: First::Yes,
            ..
        }
    ));
    script.release.lock().unwrap().send(unreachable).unwrap();
    settle(&mut dom).await;
    assert!(
        matches!(
            link(&dom),
            Link::Waiting {
                first: First::Yes,
                ..
            }
        ),
        "{:?}",
        link(&dom)
    );

    fetching.sync_all(Trigger::Manual);
    settle(&mut dom).await;
    assert!(
        matches!(
            link(&dom),
            Link::Syncing {
                first: First::Yes,
                ..
            }
        ),
        "the retry forgot nothing had ever been fetched: {:?}",
        link(&dom)
    );
    assert_eq!(script.runs.load(Ordering::SeqCst), 2);
}

/// Just the fetching, with the revision the window would bump in the context.
#[component]
fn Probe() -> Element {
    let revision = use_signal(|| 0u64);
    use_context_provider(|| revision);
    super::use_fetching(revision);
    rsx! {}
}

#[tokio::test]
async fn the_accounts_that_come_and_go_get_and_lose_their_link() {
    fn acct_later() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000ca"))
    }
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    dispatching();
    let mut dom = VirtualDom::new(Probe)
        .with_root_context(store.clone())
        .with_root_context(passer);
    dom.rebuild_in_place();
    settle(&mut dom).await;
    assert!(matches!(link(&dom), Link::Current { .. }));
    let changed = |dom: &VirtualDom| {
        dom.in_scope(ScopeId::APP, || {
            let mut revision = consume_context::<Signal<u64>>();
            revision += 1;
        })
    };

    // A new account is fetched at once, from nothing: it is the first pass.
    let manual = presets::Manual {
        imap_host: "imap.other.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.other.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("two@other.example", &manual, Utc::now());
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                acct_later().to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    store
        .put_caps(acct_later(), &preset.expected_caps, Utc::now())
        .unwrap();
    changed(&dom);
    settle(&mut dom).await;
    let added = dom.in_scope(ScopeId::APP, || fetching(&dom).link(acct_later()));
    assert!(
        matches!(
            added,
            Some(Link::Syncing {
                first: First::Yes,
                ..
            })
        ),
        "{added:?}"
    );
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        1,
        "only the new account ran"
    );

    // Gone: its link goes, and the pass it was running is cancelled and forgotten.
    store
        .connection()
        .execute(
            "DELETE FROM accounts WHERE id = ?1",
            [acct_later().to_string()],
        )
        .unwrap();
    changed(&dom);
    settle(&mut dom).await;
    assert_eq!(
        dom.in_scope(ScopeId::APP, || fetching(&dom).link(acct_later())),
        None
    );
    assert!(
        matches!(link(&dom), Link::Current { .. }),
        "the other account is untouched"
    );
}
