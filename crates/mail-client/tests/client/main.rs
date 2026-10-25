//! The client over real stores: values shared, read again when a commit touches what they read
//! and not otherwise, gathered, dropped, and heard from another process.

use chrono::{DateTime, Utc};
use mail_client::query::{Accounts, Counts};
use mail_client::{Client, ClientConfig, ClientError, Live, Query, Snapshot, Topics};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{
    Filter, LabelId, LabelOrigin, Property, ReadState, Sort, SortDir, Threading, View, ViewId,
    ViewKind,
};
use mail_store::testing::{in_memory, seed_account, seed_label};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn account(n: u128) -> AccountId {
    account_id_from_uuid(uuid::Uuid::from_u128(n))
}

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
        shown: Vec::new(),
        hover: Vec::new(),
    }
}

/// A client with threads of its own over `store`.
fn client(store: &Arc<SqliteStore>) -> Client {
    Client::open(store.clone(), ClientConfig::default())
}

/// A store on disk, so it has readers beside its writer, and the directory it is in.
fn on_disk() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    (Arc::new(store), dir)
}

/// Wait until `done` holds, or fail saying `what`.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what} did not happen");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Wait until nothing has been read for a while: settled, so a count of readings is final.
fn settled(client: &Client) {
    until("the client settling", || client.idle());
    std::thread::sleep(Duration::from_millis(250));
    until("the client settling", || client.idle());
}

/// A query that counts its readings and reads how many views there are, with what it reads as
/// part of it. Two are the same query when their names are.
#[derive(Clone)]
struct Counted {
    name: &'static str,
    runs: Arc<AtomicUsize>,
    /// Held by a reading until something is sent, when there is one.
    gate: Option<Arc<Mutex<Receiver<()>>>>,
}

impl PartialEq for Counted {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for Counted {}

impl Counted {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            runs: Arc::new(AtomicUsize::new(0)),
            gate: None,
        }
    }

    /// One whose readings each wait for a word on the sender returned.
    fn gated(name: &'static str) -> (Self, Sender<()>) {
        let (tx, rx) = channel();
        let mut query = Self::new(name);
        query.gate = Some(Arc::new(Mutex::new(rx)));
        (query, tx)
    }

    fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

impl Query for Counted {
    type Output = usize;
    const READS: Topics = Topics::VIEWS;

    fn run(&self, store: &SqliteStore, _now: DateTime<Utc>) -> Result<usize, ClientError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.lock().unwrap().recv().unwrap();
        }
        Ok(store.views()?.len())
    }
}

/// The same, reading only summaries: a write to views is nothing to it. One of its kind.
#[derive(Clone)]
struct Unrelated(Arc<AtomicUsize>);

impl PartialEq for Unrelated {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Unrelated {}

impl Query for Unrelated {
    type Output = ();
    const READS: Topics = Topics::SUMMARIES;

    fn run(&self, _store: &SqliteStore, _now: DateTime<Utc>) -> Result<(), ClientError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn value<T: Send + Sync + 'static>(live: &Live<T>) -> Option<Arc<T>> {
    match live.snapshot() {
        Snapshot::Ready { value, .. } => Some(value),
        _ => None,
    }
}

#[test]
fn two_asking_the_same_query_share_one_value_and_one_reading() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let query = Counted::new("views");

    let one = client.live(query.clone());
    let two = client.live(query.clone());
    assert_eq!(*one.wait(), 0);
    settled(&client);

    assert_eq!(query.runs(), 1);
    assert_eq!(value(&two).as_deref(), Some(&0));
}

#[test]
fn a_commit_reads_again_what_reads_what_it_touched_and_nothing_else() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let views = client.live(Counted::new("views"));
    let unrelated_runs = Arc::new(AtomicUsize::new(0));
    let _unrelated = client.live(Unrelated(unrelated_runs.clone()));
    views.wait();
    settled(&client);

    store.put_view(&view("Unread")).unwrap();

    until("the view count reading 1", || {
        value(&views).as_deref() == Some(&1)
    });
    settled(&client);
    assert_eq!(
        unrelated_runs.load(Ordering::SeqCst),
        1,
        "read again for views"
    );
}

#[test]
fn a_commit_that_said_nothing_reads_everything_again() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let runs = Arc::new(AtomicUsize::new(0));
    let _unrelated = client.live(Unrelated(runs.clone()));
    settled(&client);

    // Seeding writes through the writer without naming what it touches.
    seed_account(&store, account(1), "one@example.test");

    until("a second reading", || runs.load(Ordering::SeqCst) == 2);
}

#[test]
fn commits_that_come_together_are_read_again_once_or_twice_not_each_time() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let query = Counted::new("views");
    let views = client.live(query.clone());
    views.wait();
    settled(&client);

    for n in 0..40 {
        store.put_view(&view(&format!("View {n}"))).unwrap();
    }

    until("every view counted", || {
        value(&views).as_deref() == Some(&40)
    });
    settled(&client);
    let again = query.runs() - 1;
    assert!(
        (1..=3).contains(&again),
        "forty commits read the value again {again} times"
    );
}

#[test]
fn a_value_let_go_of_while_it_waits_is_not_read() {
    // In memory: one reading thread, which the gated query holds.
    let store = Arc::new(in_memory());
    let client = client(&store);
    let (busy, open) = Counted::gated("busy");
    let _busy = client.live(busy.clone());
    until("the busy reading to begin", || busy.runs() == 1);

    let waiting = Counted::new("waiting");
    drop(client.live(waiting.clone()));
    open.send(()).unwrap();
    settled(&client);

    assert_eq!(waiting.runs(), 0);
}

#[test]
fn a_value_that_goes_stale_while_it_is_read_is_read_again_after() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let (query, open) = Counted::gated("views");
    let views = client.live(query.clone());
    until("the first reading to begin", || query.runs() == 1);

    store.put_view(&view("During")).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    open.send(()).unwrap();
    open.send(()).unwrap();

    until("the view counted", || value(&views).as_deref() == Some(&1));
    settled(&client);
    assert_eq!(query.runs(), 2);
}

#[test]
fn an_account_removed_takes_its_labels_with_it() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let (one, two) = (account(1), account(2));
    seed_account(&store, one.clone(), "one@example.test");
    seed_account(&store, two.clone(), "two@example.test");
    seed_label(
        &store,
        LabelId::generate(),
        one.clone(),
        "Travel",
        LabelOrigin::User,
    );
    seed_label(
        &store,
        LabelId::generate(),
        two.clone(),
        "Work",
        LabelOrigin::User,
    );
    let accounts = client.live(Accounts);
    let labels = |accounts: &[mail_client::query::Account]| -> Vec<String> {
        accounts
            .iter()
            .flat_map(|account| account.labels().iter().map(|label| label.name.clone()))
            .collect()
    };
    assert_eq!(labels(&accounts.wait()), ["Travel", "Work"]);

    store.remove_account(one).unwrap();

    until("the account gone", || {
        value(&accounts).is_some_and(|now| now.len() == 1)
    });
    assert_eq!(labels(&value(&accounts).unwrap()), ["Work"]);
}

#[test]
fn counts_answer_each_filter_beside_it() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    seed_account(&store, account(1), "one@example.test");
    let unread = Filter::Read(ReadState::Unread);
    let nothing = Filter::Nothing;
    let counts = client.live(Counts(vec![unread.clone(), nothing.clone()]));

    let counted = counts.wait();

    assert_eq!(Counts::of(&counted, &nothing), Some(0));
    assert_eq!(Counts::of(&counted, &unread), Some(0));
    assert_eq!(Counts::of(&counted, &Filter::All), None, "not asked");
}

#[test]
fn another_process_committing_is_heard() {
    let (store, dir) = on_disk();
    let client = client(&store);
    let accounts = client.live(Accounts);
    assert!(accounts.wait().is_empty());

    // Another connection to the same file: what `mailo watch` is.
    let other = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    seed_account(&other, account(7), "seven@example.test");

    until("the other process's account", || {
        value(&accounts).is_some_and(|now| now.len() == 1)
    });
}

#[test]
fn an_inline_client_has_the_value_at_once_and_reads_again_when_settled() {
    let store = Arc::new(in_memory());
    let client = Client::inline(store.clone(), Arc::new(mail_core::mail::SystemClock));
    let views = client.live(Counted::new("views"));
    assert_eq!(
        value(&views).as_deref(),
        Some(&0),
        "read as it was asked for"
    );

    store.put_view(&view("Unread")).unwrap();
    assert_eq!(value(&views).as_deref(), Some(&0), "nothing until settled");
    client.settle();

    assert_eq!(value(&views).as_deref(), Some(&1));
}

#[test]
fn a_change_is_told_and_the_same_answer_is_not() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let mut views = client.live(Counted::new("views"));
    views.wait();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    // The first reading has been told already; take it.
    runtime.block_on(async {
        let _ = tokio::time::timeout(Duration::from_millis(50), views.changed()).await;
    });

    // A commit to views that leaves their count as it was: read again, and not news.
    let kept = view("Kept");
    store.put_view(&kept).unwrap();
    runtime.block_on(async {
        assert!(views.changed().await);
    });
    store.put_view(&kept).unwrap();
    settled(&client);
    let told = runtime.block_on(async {
        tokio::time::timeout(Duration::from_millis(200), views.changed())
            .await
            .is_ok()
    });
    assert!(!told, "the same count was told as a change");
}

#[test]
fn a_stopped_client_reads_nothing_more() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let query = Counted::new("views");
    let views = client.live(query.clone());
    views.wait();
    client.clone().shutdown(Duration::from_secs(2));

    store.put_view(&view("After")).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(query.runs(), 1);
    assert_eq!(value(&views).as_deref(), Some(&0));
}

#[test]
fn a_change_anywhere_is_told_to_whoever_follows_the_store() {
    let (store, dir) = on_disk();
    let client = client(&store);
    let moved = client.moved();
    let at = *moved.borrow();

    store.put_view(&view("Here")).unwrap();
    until("a commit here told", || *moved.borrow() > at);

    let at = *moved.borrow();
    let other = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    other.put_view(&view("There")).unwrap();
    until("another process's commit told", || *moved.borrow() > at);
}

/// What time a reading was made at: a query that reads only the clock.
#[derive(Clone, PartialEq, Eq)]
struct Now;

impl Query for Now {
    type Output = DateTime<Utc>;
    const READS: Topics = Topics::CLOCK;

    fn run(&self, _: &SqliteStore, now: DateTime<Utc>) -> Result<DateTime<Utc>, ClientError> {
        Ok(now)
    }
}

/// A clock that always says the same time.
struct Stopped(DateTime<Utc>);

impl mail_core::mail::Clock for Stopped {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[test]
fn a_new_clock_reads_again_what_reads_the_clock_at_its_time() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let ahead = Utc::now() + chrono::TimeDelta::days(1);
    let now = client.live(Now);
    now.wait();

    client.set_clock(Arc::new(Stopped(ahead)));

    until("the clock's time read", || {
        value(&now).as_deref() == Some(&ahead)
    });
}

#[test]
fn a_conversation_that_is_not_there_is_none_not_a_failure() {
    let store = Arc::new(in_memory());
    let client = Client::inline(store, Arc::new(mail_core::mail::SystemClock));
    let gone = client.live(mail_client::query::Conversation(
        mail_domain::ThreadId::generate(),
    ));

    assert!(matches!(gone.snapshot(), Snapshot::Ready { value, .. } if value.is_none()));
}

/// `ticket`, waited for.
fn done(ticket: mail_client::Ticket) -> Option<mail_client::Acted> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(ticket)
}

#[test]
fn a_command_is_done_off_the_callers_thread_and_says_its_generation() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    seed_account(&store, account(1), "one@example.test");
    let before = store.generation();

    let acted = done(client.act(mail_client::Command::Perform {
        ops: vec![(mail_domain::ThreadId::generate(), mail_domain::Op::Archive)],
        clock: mail_core::when::Clock::H24,
    }))
    .expect("done");

    // A conversation that is not there is not acted on, and that is all.
    assert_eq!(acted.done, mail_client::Done::Performed(Vec::new()));
    assert!(acted.generation >= before);
}

#[test]
fn what_was_given_before_a_stop_is_still_done() {
    let (store, _dir) = on_disk();
    let client = client(&store);
    let ticket = client.act(mail_client::Command::Destroy(vec![
        mail_domain::ThreadId::generate(),
    ]));
    client.clone().shutdown(Duration::from_secs(2));

    assert_eq!(
        done(ticket).map(|acted| acted.done),
        Some(mail_client::Done::Destroyed(Vec::new()))
    );
}

#[test]
fn an_inline_client_has_it_done_by_the_time_it_hands_the_ticket_back() {
    let store = Arc::new(in_memory());
    let client = Client::inline(store, Arc::new(mail_core::mail::SystemClock));
    let mut ticket = client.act(mail_client::Command::TakeBack(Vec::new()));

    assert!(ticket.now().is_some());
}

/// An IMAP account `n` in `store`, as Add Account leaves one.
fn imap_account(store: &SqliteStore, n: u128) -> AccountId {
    let manual = mail_domain::presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let address = format!("me{n}@nowhere.example");
    let preset = mail_domain::presets::manual(&address, &manual, Utc::now());
    mail_store::testing::seed_account_plan(
        store,
        account(n),
        &preset.plan.address,
        &preset.plan,
        Some(Utc::now()),
    );
    store
        .put_caps(account(n), &preset.expected_caps, Utc::now())
        .unwrap();
    account(n)
}

/// Seams that reach no server: each pass is counted and ends at once having fetched nothing.
fn counting() -> (mail_client::fetch::Seams, Arc<Mutex<Vec<AccountId>>>) {
    use mail_core::sync::report::{AccountReport, Counts, PassEnd};
    let passes = Arc::new(Mutex::new(Vec::new()));
    let counted = passes.clone();
    let seams = mail_client::fetch::Seams {
        pass: Arc::new(move |_, _, account, _| {
            counted.lock().unwrap().push(account.clone());
            Ok(vec![PassEnd::Finished(AccountReport {
                account,
                address: String::new(),
                counts: Counts::default(),
                trouble: vec![],
            })])
        }),
        listen: Arc::new(|_, _, _, _, _| {
            Err(mail_core::schedule::Lost {
                retry: mail_domain::Retry::Fatal("no watch here".to_owned()),
                why: "no watch here".to_owned(),
            })
        }),
        pushers: Arc::new(|_| Vec::new()),
        daemon: Arc::new(|| mail_core::schedule::Daemon::Absent),
        schedule: Arc::new(|| mail_core::schedule::Schedule::Window),
        folder: Arc::new(|_, _, _, _| Err("no folders here".to_owned())),
        timing: mail_core::schedule::Timing {
            beat: Duration::ZERO,
            first: Duration::ZERO,
            ..mail_core::schedule::Timing::default()
        },
    };
    (seams, passes)
}

#[test]
fn a_client_fetches_on_a_thread_of_its_own_the_accounts_it_has_and_those_added_after() {
    let (store, _dir) = on_disk();
    let first = imap_account(&store, 1);
    let client = client(&store);
    let (seams, passes) = counting();
    assert!(client.fetched().is_none(), "nothing fetches until asked");

    assert!(
        client.fetch(seams.clone()).is_none(),
        "run on its own thread"
    );
    assert!(client.fetch(seams).is_none(), "and started once");
    let fetched = client.fetched().expect("fetching");
    until("the account it had fetched", || {
        passes.lock().unwrap().contains(&first)
            && fetched
                .borrow()
                .links
                .get(&first)
                .is_some_and(|link| matches!(link, mail_core::fetch::Link::Current { .. }))
    });

    let later = imap_account(&store, 2);
    until("the account added fetched", || {
        passes.lock().unwrap().contains(&later) && fetched.borrow().links.contains_key(&later)
    });
    assert_eq!(passes.lock().unwrap().len(), 2, "each fetched once");
    client.shutdown(Duration::from_secs(1));
}

#[test]
fn a_send_that_comes_due_asks_its_account_for_a_pass_then() {
    let (store, _dir) = on_disk();
    let one = imap_account(&store, 1);
    let client = client(&store);
    let (mut seams, passes) = counting();
    // Not the first pass's to carry: that one is a while off.
    seams.timing.beat = Duration::from_secs(3600);
    client.fetch(seams);
    std::thread::sleep(Duration::from_millis(100));
    assert!(passes.lock().unwrap().is_empty());

    let due = Utc::now() + chrono::TimeDelta::milliseconds(300);
    store
        .enqueue(
            one.clone(),
            mail_domain::RemoteIntent::Send {
                draft: mail_domain::DraftId::generate(),
                raw: mail_domain::BlobId::generate(),
                mail_from: "me1@nowhere.example".to_owned(),
                rcpt_to: vec!["you@example.test".to_owned()],
            },
            &mail_domain::Patch {
                id: mail_domain::ChangeId::generate(),
                changes: Vec::new(),
            },
            due,
        )
        .unwrap();
    until("the account asked for a pass", || {
        passes.lock().unwrap().contains(&one)
    });
    assert!(Utc::now() >= due, "not before the send was due");
    client.shutdown(Duration::from_secs(1));
}
