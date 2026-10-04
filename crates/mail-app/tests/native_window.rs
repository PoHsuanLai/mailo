//! A conversation in a window of its own, driven in the real window on Blitz
//! (`ds_harness::Harness`): the reader's menu, a row's context menu and Shift+Enter ask for the
//! window, and the window's root draws the reader alone and follows what the main window does to
//! the conversation, and the other way round.
//!
//! quire's harness has no event loop to open a second window on (`open_window` answers
//! `NoHost` there), so the main window is handed a recorder as its `Windows` and the asks are
//! read from it; the second window is its root in a harness of its own, over the same store and
//! the same `Revisions`, as quire hands a real second window the first one's root contexts.
//!
//! Every store is seeded in a `TempDir`, and no window is handed directories, so nothing here
//! writes a file anywhere else.

use ds::base::press::PointerButton;
use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome, RootContexts};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use mail_app::ui::native::{Ask, MessageOpen, OpenWindow, Revisions, Windows};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The size quire opens a conversation's window at.
const WINDOW: Viewport = Viewport {
    width: 760,
    height: 720,
    scale_percent: 100,
};

/// The seeded inbox, newest first, as the list draws it: (sender, subject).
const INBOX: [(&str, &str); 3] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// What "Open in new window" asked for, in order.
#[derive(Default)]
struct Recorder(Mutex<Vec<Ask>>);

impl OpenWindow for Recorder {
    fn open(&self, ask: Ask) {
        self.0.lock().unwrap().push(ask);
    }
}

impl Recorder {
    fn asked(&self) -> Vec<Ask> {
        self.0.lock().unwrap().clone()
    }
}

/// A store in `dir` with one account, its identity and capabilities, and [`INBOX`].
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
        let caps = AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: FolderRoles::default(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Yes,
            connections: ConnectionBudget::default(),
            observed_at: chrono::Utc::now(),
        };
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&caps).unwrap()],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <window{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
        );
        absorb(
            &store,
            ACCOUNT,
            MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("window{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// The conversation whose subject is `subject`, as the store has it.
fn thread(store: &SqliteStore, subject: &str) -> ThreadSummary {
    let query = Query {
        filter: Filter::Account(ACCOUNT),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 100,
        },
    };
    store
        .threads(&query, chrono::Utc::now())
        .unwrap()
        .items
        .into_iter()
        .find(|thread| thread.subject == subject)
        .unwrap_or_else(|| panic!("no conversation {subject:?}"))
}

/// What every window of the app is handed: the store, no directories, a printer that never
/// opens a dialog, and the app's one `Revisions`.
fn window_contexts(store: &Arc<SqliteStore>, revisions: &Revisions) -> RootContexts {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(revisions.clone())
}

/// The main window over `store`, first frame drawn, its "Open in new window" going to the
/// recorder it returns.
fn main_window(store: &Arc<SqliteStore>, revisions: &Revisions) -> (Harness, Arc<Recorder>) {
    main_window_on(Clock::Virtual, store, revisions)
}

/// [`main_window`] on `clock`. Two windows in one test share the thread's one installed clock, so
/// a test that drives both runs them on the wall clock; a test with one window runs it virtually.
fn main_window_on(
    clock: Clock,
    store: &Arc<SqliteStore>,
    revisions: &Revisions,
) -> (Harness, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let windows: Arc<dyn OpenWindow> = recorder.clone();
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(clock)
        .with_contexts(window_contexts(store, revisions).with(Windows(windows)));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == INBOX.len());
    (harness, recorder)
}

/// `thread`'s own window over `store`, as quire opens it, first frame drawn.
fn message_window(store: &Arc<SqliteStore>, revisions: &Revisions, thread: ThreadId) -> Harness {
    message_window_on(Clock::Virtual, store, revisions, thread)
}

/// [`message_window`] on `clock`; see [`main_window_on`].
fn message_window_on(
    clock: Clock,
    store: &Arc<SqliteStore>,
    revisions: &Revisions,
    thread: ThreadId,
) -> Harness {
    let config = HarnessConfig::new(WINDOW)
        .with_net(NetPolicy::Local)
        .with_clock(clock)
        .with_contexts(window_contexts(store, revisions).with(MessageOpen(thread)));
    let mut harness = Harness::new(mail_app::ui::native::message_root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".reader-head h2") == 1);
    harness
}

/// The `n`th row of the list (1-based).
fn row(n: usize) -> String {
    format!(".list .ds-list > .ds-list-item:nth-child({n})")
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Where a person reads the `n`th row: the start of its subject line, clear of the hover strip.
fn on_row(harness: &Harness, n: usize) -> Point {
    let subject = format!("{} .ds-thread-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    }
}

fn click_row(harness: &mut Harness, n: usize) {
    let at = on_row(harness, n);
    harness.click(at);
    harness.advance(ms(300));
}

/// Wait for the one menu row, then press it where it is drawn.
fn press_open_in_window(harness: &mut Harness) {
    let item = ".ds-menu-item";
    settle_until(harness, |h| {
        h.centre(item).is_some_and(|at| h.hits(at, item))
    });
    assert_eq!(
        harness
            .text_of(item)
            .as_deref()
            .map(str::trim)
            .map(|t| t.starts_with("Open in new window")),
        Some(true),
        "{:?}",
        harness.text_of(item)
    );
    harness.click(centre(harness, item));
    // The menu blinks the picked row and acts on it as it closes, which takes real time on a
    // slow runner: a fixed advance can end before the ask is made.
    settle_until(harness, |h| h.count(".ds-menu") == 0);
}

fn asked_for(store: &SqliteStore, subject: &str) -> Vec<Ask> {
    vec![Ask {
        thread: thread(store, subject).id,
        title: subject.to_owned(),
    }]
}

#[test]
fn the_reader_s_menu_asks_for_the_open_conversation_in_a_window_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(&store, &Revisions::new());
    click_row(&mut harness, 2);
    assert_eq!(recorder.asked(), Vec::<Ask>::new());

    harness.click(centre(&harness, ".reader-head [*|aria-label=\"More\"]"));
    press_open_in_window(&mut harness);
    assert_eq!(recorder.asked(), asked_for(&store, INBOX[1].1));
    assert_eq!(harness.count(".ds-menu"), 0, "the pick left the menu open");
}

#[test]
fn a_row_s_context_menu_asks_for_that_conversation_in_a_window_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(&store, &Revisions::new());
    // Nothing open: the row right-clicked is the one asked for.
    let at = on_row(&harness, 3);
    harness.press(at, PointerButton::Secondary);
    press_open_in_window(&mut harness);
    assert_eq!(recorder.asked(), asked_for(&store, INBOX[2].1));
}

#[test]
fn shift_enter_asks_for_the_focused_conversation_in_a_window_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(&store, &Revisions::new());
    click_row(&mut harness, 1);
    // Enter alone opens no window.
    harness.key(Key::Enter);
    harness.advance(ms(100));
    assert_eq!(recorder.asked(), Vec::<Ask>::new());
    harness.chord(&[Key::Shift], Key::Enter);
    settle_until(&mut harness, |_| {
        recorder.asked() == asked_for(&store, INBOX[0].1)
    });
}

#[test]
fn the_window_s_root_draws_the_reader_for_its_conversation_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let lunch = thread(&store, INBOX[2].1).id;
    let harness = message_window(&store, &Revisions::new(), lunch);

    assert_eq!(
        harness.text_of(".reader-head h2").as_deref(),
        Some(INBOX[2].1)
    );
    let frame = harness
        .frame("article.frame iframe.html")
        .expect("the frame has a document");
    assert!(
        frame.html().contains("The body of Lunch on Thursday."),
        "{}",
        frame.html()
    );
    assert_eq!(harness.count(".ds-list"), 0, "the window drew a list");
    assert_eq!(harness.count(".side"), 0, "the window drew the sidebar");
    // The window is the page: no peek, and no second "Open in new window" from inside it.
    for tool in ["Side peek", "Centre peek", "Full page", "More"] {
        assert_eq!(
            harness.count(&format!(".reader-head [*|aria-label=\"{tool}\"]")),
            0,
            "{tool} is drawn"
        );
    }
    assert_eq!(
        harness.count(".left-note"),
        0,
        "nothing has happened to it yet"
    );
    // Its consent starts empty, whatever the other window has granted.
    assert_eq!(harness.count(".consent"), 0);
}

#[test]
fn the_window_follows_what_the_main_window_does_to_its_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let revisions = Revisions::new();
    let invoice = thread(&store, INBOX[1].1).id;
    let (mut main, _) = main_window_on(Clock::Wall, &store, &revisions);
    let mut window = message_window_on(Clock::Wall, &store, &revisions, invoice);

    // Muted in the main window: the second window's reader says so.
    click_row(&mut main, 2);
    main.key(Key::Char('m'));
    settle_until(&mut main, |_| {
        thread(&store, INBOX[1].1).mute == Mute::Muted
    });
    settle_until(&mut window, |h| h.count(".reader-head .muted-note") == 1);

    // Archived in the main window: the second window says where it went.
    main.key(Key::Char('e'));
    settle_until(&mut main, |_| {
        !thread(&store, INBOX[1].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| {
        h.text_of(".left-note")
            .is_some_and(|note| note.contains("Archived"))
    });

    // Taken back in the main window: the note goes.
    main.chord(&[Key::Ctrl], Key::Char('z'));
    settle_until(&mut main, |_| {
        thread(&store, INBOX[1].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| h.count(".left-note") == 0);
}

#[test]
fn an_archive_in_the_window_reaches_the_main_list_and_its_own_ctrl_z_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let revisions = Revisions::new();
    let lunch = thread(&store, INBOX[2].1).id;
    let (mut main, _) = main_window_on(Clock::Wall, &store, &revisions);
    let mut window = message_window_on(Clock::Wall, &store, &revisions, lunch);

    window.key(Key::Char('e'));
    settle_until(&mut window, |_| {
        !thread(&store, INBOX[2].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    // The window says so, with the toast whose undo is this window's.
    settle_until(&mut window, |h| {
        h.text_of(".left-note")
            .is_some_and(|note| note.contains("Archived"))
    });
    assert!(
        window
            .text_of(".ds-toast")
            .unwrap_or_default()
            .contains("Archived"),
        "{:?}",
        window.text_of(".ds-toast")
    );
    // And the main window's inbox lets the row go.
    settle_until(&mut main, |h| {
        h.count(".list .ds-thread") == INBOX.len() - 1
    });

    window.chord(&[Key::Ctrl], Key::Char('z'));
    settle_until(&mut window, |_| {
        thread(&store, INBOX[2].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| h.count(".left-note") == 0);
    settle_until(&mut main, |h| h.count(".list .ds-thread") == INBOX.len());
}

/// A banner's click, or `mailo open` from a terminal, reaches the running window over the session
/// bus as a request (`ui/handoff`); here the bus is left out and the request is pushed straight
/// into the window's queue, and the window opens that conversation itself, where the person is
/// reading, instead of in a window of its own. (The raise with the click's token is quire's
/// event loop, which the harness has none of.)
#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn a_request_from_outside_opens_the_conversation_in_the_running_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (requests, queue) = mail_app::ui::native::Requests::local();
    let recorder = Arc::new(Recorder::default());
    let windows: Arc<dyn OpenWindow> = recorder.clone();
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(
            window_contexts(&store, &Revisions::new())
                .with(Windows(windows))
                .with(requests),
        );
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == INBOX.len());
    assert_eq!(
        harness.count(".reader-head h2"),
        0,
        "a reader opened unasked"
    );

    let invoice = thread(&store, INBOX[1].1).id;
    queue
        .send(mail_app::ui::native::Request::Thread {
            thread: invoice,
            token: mail_app::ui::native::ActivationToken::new("xdg-activation-1"),
        })
        .unwrap();
    settle_until(&mut harness, |h| h.count(".reader-head h2") == 1);
    let heading = harness.text_of(".reader-head h2").unwrap_or_default();
    assert!(heading.contains(INBOX[1].1), "{heading:?}");
    assert!(
        recorder.asked().is_empty(),
        "a window of its own was opened: {:?}",
        recorder.asked()
    );

    // A request to raise the window itself changes nothing.
    queue
        .send(mail_app::ui::native::Request::Activate { token: None })
        .unwrap();
    harness.advance(ms(300));
    assert_eq!(
        harness.text_of(".reader-head h2").unwrap_or_default(),
        heading
    );
    assert!(recorder.asked().is_empty());
}
