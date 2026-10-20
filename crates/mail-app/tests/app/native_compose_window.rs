//! A new message in a window of its own, driven in the real window on Blitz
//! (`ds_harness::Harness`): New Message, ⌘N and ⌘K's New Message ask for a compose window and leave
//! the reader as it was; a reply stays under its thread; the compose window parks into the main
//! window, where ⌘K's Recent brings it back, and hands its page back for Open in Main Window; and
//! neither window draws a Focus control.
//!
//! quire's harness has no event loop to open a second window on, so the main window is handed a
//! recorder as its `ComposeWindows` and the asks are read from it; the compose window is its root
//! in a harness of its own, over the same store, the same `Revisions` and the same hub, as the
//! launched app hands every window the first one's root contexts. Two windows in one test share
//! the thread's one installed clock, so a test that drives both runs them on the wall clock.
//!
//! Every store is seeded in a `TempDir`, and no window is handed directories, so nothing here
//! writes a file anywhere else.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::one_message;
use drive::{Drive, PRIMARY};
use mail_app::ui::native::{
    ComposeAsk, ComposeHub, ComposeOpen, ComposeWindows, OpenCompose, Revisions,
};
use mail_core::SqliteStore;
use mail_domain::DraftId;
use one_message::{SUBJECT, seeded};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The size quire opens a compose window at.
const WINDOW: Viewport = Viewport {
    width: 640,
    height: 560,
    scale_percent: 100,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The compose windows asked for, in order.
#[derive(Default)]
struct Recorder(Mutex<Vec<ComposeAsk>>);

impl OpenCompose for Recorder {
    fn open(&self, ask: ComposeAsk) {
        self.0.lock().unwrap().push(ask);
    }
}

impl Recorder {
    fn asked(&self) -> Vec<ComposeAsk> {
        self.0.lock().unwrap().clone()
    }
}

/// What every window of the app is handed: the store, no directories, a printer that never
/// opens a dialog, the app's one `Revisions` and its one hub.
fn window_contexts(
    store: &Arc<SqliteStore>,
    revisions: &Revisions,
    hub: &ComposeHub,
) -> RootContexts {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(revisions.clone())
    .with(hub.clone())
}

/// The main window over `store` on `clock`, first frame drawn, its compose windows going to the
/// recorder it returns.
fn main_window(
    clock: Clock,
    store: &Arc<SqliteStore>,
    revisions: &Revisions,
    hub: &ComposeHub,
) -> (Harness, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let windows: Arc<dyn OpenCompose> = recorder.clone();
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(clock)
        .with_contexts(window_contexts(store, revisions, hub).with(ComposeWindows(windows)));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == 1);
    (harness, recorder)
}

/// `draft`'s compose window, as quire opens it, its composer drawn.
fn compose_window(
    store: &Arc<SqliteStore>,
    revisions: &Revisions,
    hub: &ComposeHub,
    draft: DraftId,
) -> Harness {
    let config = HarnessConfig::new(WINDOW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Wall)
        .with_contexts(window_contexts(store, revisions, hub).with(ComposeOpen(draft)));
    let mut harness = Harness::new(mail_app::ui::native::compose_root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    harness
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn labelled(label: &str) -> String {
    format!("[*|aria-label=\"{label}\"]")
}

/// Open the one conversation, and wait until the reader draws it.
fn open_conversation(harness: &mut Harness) {
    let subject = ".list .ds-list-item[*|aria-posinset=\"1\"] .ds-thread-sub";
    let rect = harness
        .rect(subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    settle_until(harness, |h| {
        h.text_of(".reader-head h2")
            .is_some_and(|title| title.trim() == SUBJECT)
    });
}

/// The reader is drawn as it was: the conversation's title, and no composer page.
fn assert_reader_kept(harness: &Harness, what: &str) {
    assert_eq!(
        harness.count(".cpage"),
        0,
        "{what} drew a composer in the reader"
    );
    assert_eq!(
        harness
            .text_of(".reader-head h2")
            .map(|title| title.trim().to_owned()),
        Some(SUBJECT.to_owned()),
        "{what} took the reader's conversation away"
    );
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    harness.advance(ms(100));
}

/// The open menu's row whose label is `name`, once it is drawn where a press lands.
fn menu_row(harness: &mut Harness, name: &str) -> String {
    let list = ".ds-menu-list";
    let row = |h: &Harness| {
        (1..=h.count(&format!("{list} > *")))
            .map(|n| format!("{list} > :nth-child({n})"))
            .find(|item| {
                h.text_of(&format!("{item} .ds-menu-label"))
                    .is_some_and(|label| label.trim() == name)
            })
    };
    settle_until(harness, |h| {
        row(h).is_some_and(|item| h.centre(&item).is_some_and(|at| h.hits(at, &item)))
    });
    row(harness).unwrap_or_else(|| panic!("no {name} in the menu:\n{}", harness.html()))
}

/// Press the open menu's row named `name`, and wait for the menu to go.
fn pick(harness: &mut Harness, name: &str) {
    let item = menu_row(harness, name);
    harness.click(centre(harness, &item));
    settle_until(harness, |h| h.count(".ds-menu") == 0);
}

/// Neither a bar above the page nor a control named Focus.
fn assert_no_focus(harness: &Harness, what: &str) {
    assert_eq!(harness.count(".c-top"), 0, "{what} draws the Focus bar");
    assert_eq!(
        harness.count(&labelled("Focus")),
        0,
        "{what} draws a Focus control"
    );
}

/// New Message, ⌘N and ⌘K's New Message each ask for a compose window on a draft of its own, titled
/// New Message, and the reader keeps the conversation it had open.
#[test]
fn every_way_to_a_new_message_asks_for_a_compose_window_and_keeps_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(
        Clock::Virtual,
        &store,
        &Revisions::new(),
        &ComposeHub::new(),
    );
    open_conversation(&mut harness);

    harness.click(centre(&harness, &labelled("New Message")));
    settle_until(&mut harness, |_| recorder.asked().len() == 1);
    harness.advance(ms(300));
    assert_reader_kept(&harness, "New Message");

    harness.chord(&[PRIMARY], Key::Char('n'));
    settle_until(&mut harness, |_| recorder.asked().len() == 2);
    harness.advance(ms(300));
    assert_reader_kept(&harness, "⌘N");

    harness.chord(&[PRIMARY], Key::Char('k'));
    settle_until(&mut harness, |h| h.is_focused(".ds-search-card input"));
    type_text(&mut harness, "new message");
    // Enter runs the highlighted row once the panel has answered: the text starts New Message.
    settle_until(&mut harness, |h| {
        h.text_of(".ds-search-card .ds-menu-item[*|data-selected=true] .ds-menu-title")
            .is_some_and(|title| title.trim() == "New Message")
    });
    harness.key(Key::Enter);
    settle_until(&mut harness, |_| recorder.asked().len() == 3);
    settle_until(&mut harness, |h| h.count(".ds-search-card") == 0);
    assert_reader_kept(&harness, "⌘K's New Message");

    let asked = recorder.asked();
    for ask in &asked {
        assert_eq!(ask.title, "New Message", "{ask:?}");
    }
    let drafts: std::collections::HashSet<DraftId> = asked.iter().map(|ask| ask.draft).collect();
    assert_eq!(drafts.len(), 3, "two asks shared a draft: {asked:?}");
    assert_no_focus(&harness, "the main window");
}

/// A reply opens under its thread in the main window, and asks for no window.
#[test]
fn a_reply_stays_under_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let (mut harness, recorder) = main_window(
        Clock::Virtual,
        &store,
        &Revisions::new(),
        &ComposeHub::new(),
    );
    open_conversation(&mut harness);

    harness.key(Key::Char('r'));
    settle_until(&mut harness, |h| h.count(".inline-reply .c-body") == 1);
    assert_eq!(recorder.asked(), Vec::new(), "a reply asked for a window");
    assert_no_focus(&harness, "a reply");
}

/// Without a way to open a window (no recorder, no event loop), a new message is drawn in place,
/// as Open in Main Window would put it.
#[test]
fn with_no_window_to_open_a_new_message_is_drawn_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(window_contexts(
            &store,
            &Revisions::new(),
            &ComposeHub::new(),
        ));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
}

/// The compose window draws the composer alone and no Focus control. Esc parks the draft: the
/// window's composer closes, the main window's sidebar menu lists the draft under Drafts, and
/// picking it there asks for its window again.
#[test]
fn esc_in_a_compose_window_parks_the_draft_in_the_main_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let revisions = Revisions::new();
    let hub = ComposeHub::new();
    let (mut main, recorder) = main_window(Clock::Wall, &store, &revisions, &hub);
    main.chord(&[PRIMARY], Key::Char('n'));
    settle_until(&mut main, |_| recorder.asked().len() == 1);
    let draft = recorder.asked()[0].draft;

    let mut window = compose_window(&store, &revisions, &hub, draft);
    assert_eq!(
        window.count(".list"),
        0,
        "the compose window draws the list"
    );
    assert_no_focus(&window, "the compose window");
    window.click(centre(&window, ".cpage .c-title input"));
    window.advance(ms(100));
    type_text(&mut window, "Lunch plans");

    window.key(Key::Escape);
    settle_until(&mut window, |h| h.count(".cpage") == 0);

    // The main window takes the parked page: ⌘K with nothing typed lists it under Recent, and
    // Return brings it back.
    main.advance(ms(300));
    main.chord(&[PRIMARY], Key::Char('k'));
    settle_until(&mut main, |h| {
        h.is_focused(".ds-search-card input") && h.html().contains("Lunch plans")
    });
    main.key(Key::Enter);
    settle_until(&mut main, |_| recorder.asked().len() == 2);
    assert_eq!(
        recorder.asked()[1],
        ComposeAsk {
            draft,
            title: "Lunch plans".to_owned(),
        },
        "reopening the parked draft"
    );
    assert_eq!(
        main.count(".cpage"),
        0,
        "the reopened draft was drawn in place"
    );
}

/// ⋯ › Open in Main Window: the page, as it was, goes to the main window's reader column, and
/// the compose window's composer closes.
#[test]
fn open_in_main_window_moves_the_page_to_the_main_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let revisions = Revisions::new();
    let hub = ComposeHub::new();
    let (mut main, recorder) = main_window(Clock::Wall, &store, &revisions, &hub);
    main.chord(&[PRIMARY], Key::Char('n'));
    settle_until(&mut main, |_| recorder.asked().len() == 1);
    let draft = recorder.asked()[0].draft;

    let mut window = compose_window(&store, &revisions, &hub, draft);
    window.click(centre(&window, ".cpage .c-title input"));
    window.advance(ms(100));
    type_text(&mut window, "Moving over");

    window.click(centre(
        &window,
        &format!(".cpage .c-foot {}", labelled("More")),
    ));
    pick(&mut window, "Open in Main Window");
    settle_until(&mut window, |h| h.count(".cpage") == 0);

    settle_until(&mut main, |h| h.count(".cpage .c-body") == 1);
    assert_eq!(
        main.attr(".cpage .c-title input", "value").as_deref(),
        Some("Moving over"),
        "the page lost what was typed"
    );
    assert_eq!(
        recorder.asked().len(),
        1,
        "Open in Main Window asked for a window"
    );
    assert_no_focus(&main, "the moved page");
}
