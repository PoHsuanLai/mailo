//! A conversation in a window of its own, driven in the real window on Blitz
//! (`ds_harness::Harness`): the reader's menu, a row's context menu and Shift+Enter ask for the
//! window, and the window's root draws the reader alone and follows what the main window does to
//! the conversation, and the other way round.
//!
//! quire's harness has no event loop to open a second window on (`open_window` answers
//! `NoHost` there), so the main window is handed a recorder as its `Windows` and the asks are
//! read from it; the second window is its root in a harness of its own, over the same store, as
//! quire hands a real second window the first one's root contexts.
//!
//! Every store is seeded in a `TempDir`, and no window is handed directories, so nothing here
//! writes a file anywhere else.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::native::item;
use crate::native::ms;
use crate::store::acct_account;
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_app::ui::native::{Ask, MessageOpen, OpenWindow, Windows};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
    seeded_with(
        dir,
        Seed {
            inbox: &INBOX,
            prefix: "window",
            ..Seed::default()
        },
    )
}

/// The conversation whose subject is `subject`, as the store has it.
fn thread(store: &SqliteStore, subject: &str) -> ThreadSummary {
    let query = Query {
        filter: Filter::Account(acct_account()),
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

/// What every window of the app is handed: the store, no directories, and a printer that never
/// opens a dialog.
fn window_contexts(store: &Arc<SqliteStore>) -> RootContexts {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    window_contexts_printing(store, printer)
}

/// [`window_contexts`] with `printer`, for a test that reads what the window prints.
fn window_contexts_printing(
    store: &Arc<SqliteStore>,
    printer: mail_app::ui::native::Printer,
) -> RootContexts {
    mail_app::ui::native::contexts(
        mail_client::Client::open(Arc::clone(store), mail_client::ClientConfig::default()),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
}

/// The main window over `store`, first frame drawn, its "Open in new window" going to the
/// recorder it returns.
fn main_window(store: &Arc<SqliteStore>) -> (Harness, Arc<Recorder>) {
    main_window_on(Clock::Virtual, store)
}

/// [`main_window`] on `clock`. Two windows in one test share the thread's one installed clock, so
/// a test that drives both runs them on the wall clock; a test with one window runs it virtually.
fn main_window_on(clock: Clock, store: &Arc<SqliteStore>) -> (Harness, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let windows: Arc<dyn OpenWindow> = recorder.clone();
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(clock)
        .with_contexts(window_contexts(store).with(Windows(windows)));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == INBOX.len());
    (harness, recorder)
}

/// `thread`'s own window over `store`, as quire opens it, first frame drawn.
fn message_window(store: &Arc<SqliteStore>, thread: ThreadId) -> Harness {
    message_window_on(Clock::Virtual, store, thread)
}

/// [`message_window`] on `clock`; see [`main_window_on`].
fn message_window_on(clock: Clock, store: &Arc<SqliteStore>, thread: ThreadId) -> Harness {
    let config = HarnessConfig::new(WINDOW)
        .with_net(NetPolicy::Local)
        .with_clock(clock)
        .with_contexts(window_contexts(store).with(MessageOpen(thread)));
    let mut harness = Harness::new(mail_app::ui::native::message_root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".reader-head h2") == 1);
    harness
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Where a person reads the `n`th row: the start of its subject line, clear of the hover strip.
fn on_row(harness: &Harness, n: usize) -> Point {
    let subject = format!("{} .ds-thread-sub", item(n));
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

/// Wait for the menu's Open in new window, then press it where it is drawn.
fn press_open_in_window(harness: &mut Harness) {
    let row = |h: &Harness| {
        (1..=h.count(".ds-menu-list > *")).find(|n| {
            h.text_of(&format!(".ds-menu-list > :nth-child({n}) .ds-menu-label"))
                .is_some_and(|name| name.trim() == "Open in new window")
        })
    };
    settle_until(harness, |h| {
        row(h).is_some_and(|n| {
            let item = format!(".ds-menu-list > :nth-child({n})");
            h.centre(&item).is_some_and(|at| h.hits(at, &item))
        })
    });
    let n = row(harness).unwrap_or_else(|| panic!("no Open in new window:\n{}", harness.html()));
    harness.click(centre(harness, &format!(".ds-menu-list > :nth-child({n})")));
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

/// Two ways to ask for a conversation in a window of its own, each asking once: the reader's
/// menu asks for the open one; Shift+Enter asks for the focused one, and Enter alone opens no
/// window. A row's menu has no window row: the reader's ⋯ holds it.
#[test]
fn the_reader_s_menu_and_shift_enter_ask_for_a_window_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(&store);
    let mut want = Vec::<Ask>::new();

    // The reader's ⋯, View, its window row.
    click_row(&mut harness, 2);
    assert_eq!(recorder.asked(), want, "opening a row asked for a window");
    harness.click(centre(&harness, ".reader-head [*|aria-label=\"More\"]"));
    harness.advance(ms(300));
    crate::row_menu::press_menu_item(&mut harness, "View");
    press_open_in_window(&mut harness);
    want.extend(asked_for(&store, INBOX[1].1));
    assert_eq!(recorder.asked(), want, "the reader's menu");
    assert_eq!(harness.count(".ds-menu"), 0, "the pick left the menu open");

    // Shift+Enter.
    click_row(&mut harness, 1);
    // Enter alone opens no window.
    harness.key(Key::Enter);
    harness.advance(ms(100));
    assert_eq!(recorder.asked(), want, "Enter alone asked for a window");
    harness.chord(&[Key::Shift], Key::Enter);
    want.extend(asked_for(&store, INBOX[0].1));
    settle_until(&mut harness, |_| recorder.asked() == want);
}

#[test]
fn the_window_s_root_draws_the_reader_for_its_conversation_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let lunch = thread(&store, INBOX[2].1).id;
    let harness = message_window(&store, lunch);

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
    assert_eq!(
        harness.count(".ds-list, .ds-virtual-list"),
        0,
        "the window drew a list"
    );
    assert_eq!(harness.count(".side"), 0, "the window drew the sidebar");
    // The window is the page: no View, so no peek and no second "Open in new window".
    for tool in ["View", "Side peek", "Centre peek", "Full page"] {
        assert_eq!(
            harness.count(&format!(".reader-head [*|aria-label=\"{tool}\"]")),
            0,
            "{tool} is drawn"
        );
    }
    assert_eq!(
        harness.count(".reader > .ds-inline-banner"),
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
    let invoice = thread(&store, INBOX[1].1).id;
    let (mut main, _) = main_window_on(Clock::Wall, &store);
    let mut window = message_window_on(Clock::Wall, &store, invoice);

    click_row(&mut main, 2);

    // Archived in the main window: the second window says where it went.
    main.key(Key::Char('e'));
    settle_until(&mut main, |_| {
        !thread(&store, INBOX[1].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| {
        h.text_of(".reader > .ds-inline-banner .ds-inline-banner-body")
            .is_some_and(|note| note.contains("Archived"))
    });

    // Taken back in the main window: the note goes.
    main.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut main, |_| {
        thread(&store, INBOX[1].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| h.count(".reader > .ds-inline-banner") == 0);
}

#[test]
fn an_archive_in_the_window_reaches_the_main_list_and_its_own_ctrl_z_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let lunch = thread(&store, INBOX[2].1).id;
    let (mut main, _) = main_window_on(Clock::Wall, &store);
    let mut window = message_window_on(Clock::Wall, &store, lunch);

    window.key(Key::Char('e'));
    settle_until(&mut window, |_| {
        !thread(&store, INBOX[2].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    // The window says so, with the toast whose undo is this window's.
    settle_until(&mut window, |h| {
        h.text_of(".reader > .ds-inline-banner .ds-inline-banner-body")
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

    window.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut window, |_| {
        thread(&store, INBOX[2].1)
            .mailboxes
            .contains(MailboxRole::Inbox)
    });
    settle_until(&mut window, |h| h.count(".reader > .ds-inline-banner") == 0);
    settle_until(&mut main, |h| h.count(".list .ds-thread") == INBOX.len());
}

/// ⌘P prints the conversation in a window of its own as it does in the main one: the print
/// dialog is asked for the open conversation. The key is the platform's primary one, which on
/// the harness's keymap (our desktop's) is Command; Ctrl alone prints nothing, where the window
/// once tested Ctrl only and left ⌘P dead.
#[test]
fn command_p_prints_the_conversation_in_a_window_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let lunch = thread(&store, INBOX[2].1).id;
    let dialogs = Arc::new(AtomicUsize::new(0));
    let printer = mail_app::ui::native::Printer::with_dialog({
        let dialogs = Arc::clone(&dialogs);
        move |_, _| {
            dialogs.fetch_add(1, Ordering::SeqCst);
            Ok(PrintOutcome::Cancelled)
        }
    });
    let config = HarnessConfig::new(WINDOW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Wall)
        .with_contexts(window_contexts_printing(&store, printer).with(MessageOpen(lunch)));
    let mut window = Harness::new(mail_app::ui::native::message_root, config);
    window.advance(ms(300));
    settle_until(&mut window, |h| h.count(".reader-head h2") == 1);

    // Ctrl+P is not the print chord on this keymap.
    window.chord(&[Key::Ctrl], Key::Char('p'));
    window.advance(ms(500));
    assert_eq!(dialogs.load(Ordering::SeqCst), 0, "Ctrl+P printed");

    window.chord(&[PRIMARY], Key::Char('p'));
    settle_until(&mut window, |_| dialogs.load(Ordering::SeqCst) == 1);
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
            window_contexts(&store)
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
