//! A new message and a draft, written in the main window's reader column, driven in the real
//! window on Blitz (`ds_harness::Harness`): New Message, ⌘N and ⌘K's New Message draw the composer
//! there; a reply stays under its thread; Esc parks a written draft and ⌘K's Recent brings it back
//! in place; and a draft in Drafts opens in place with its row selected, its menu deleting it with
//! an undo.
//!
//! Every store is seeded in a `TempDir`, and no window is handed directories, so nothing here
//! writes a file anywhere else.

use ds::base::press::PointerButton;
use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::one_message;
use drive::{Drive, PRIMARY};
use mail_core::{SqliteStore, Store};
use one_message::{SUBJECT, account, seeded};
use std::sync::Arc;
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The main window over `store`, first frame drawn.
fn main_window(store: &Arc<SqliteStore>) -> Harness {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == 1);
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

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    harness.advance(ms(100));
}

/// The composer is drawn in the reader column, and Esc (nothing typed) closes it.
fn composes_here_then_closes(harness: &mut Harness, what: &str) {
    settle_until(harness, |h| h.count(".cpage .c-body") == 1);
    assert_eq!(harness.count(".cpage"), 1, "{what} drew no composer here");
    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(".cpage") == 0);
}

/// New Message, ⌘N and ⌘K's New Message each draw the composer in the reader column. Closed with
/// nothing in it, each leaves no draft behind.
#[test]
fn every_way_to_a_new_message_composes_in_the_reader_column() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = main_window(&store);
    open_conversation(&mut harness);

    harness.click(centre(&harness, &labelled("New Message")));
    composes_here_then_closes(&mut harness, "New Message");

    harness.chord(&[PRIMARY], Key::Char('n'));
    composes_here_then_closes(&mut harness, "⌘N");

    harness.chord(&[PRIMARY], Key::Char('k'));
    settle_until(&mut harness, |h| h.is_focused(".ds-search-card input"));
    type_text(&mut harness, "new message");
    // Enter runs the highlighted row once the panel has answered: the text starts New Message.
    settle_until(&mut harness, |h| {
        h.text_of(".ds-search-card .ds-menu-item[*|data-selected=true] .ds-menu-title")
            .is_some_and(|title| title.trim() == "New Message")
    });
    harness.key(Key::Enter);
    settle_until(&mut harness, |h| h.count(".ds-search-card") == 0);
    composes_here_then_closes(&mut harness, "⌘K's New Message");

    assert_eq!(
        store.drafts(account()).map(|drafts| drafts.len()).ok(),
        Some(0),
        "an empty composer left a draft"
    );
}

/// A reply opens under its thread.
#[test]
fn a_reply_stays_under_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = main_window(&store);
    open_conversation(&mut harness);

    harness.key(Key::Char('r'));
    settle_until(&mut harness, |h| h.count(".inline-reply .c-body") == 1);
    assert_eq!(harness.count(".cpage"), 0, "a reply drew a page of its own");
}

/// Esc parks a draft with something in it; ⌘K with nothing typed lists it under Recent by its
/// subject, and Return brings it back here as it was.
#[test]
fn esc_parks_a_written_draft_and_recent_brings_it_back_here() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = main_window(&store);
    harness.chord(&[PRIMARY], Key::Char('n'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    harness.click(centre(&harness, ".cpage .c-title input"));
    harness.advance(ms(100));
    type_text(&mut harness, "Lunch plans");
    harness.key(Key::Escape);
    settle_until(&mut harness, |h| h.count(".cpage") == 0);

    harness.chord(&[PRIMARY], Key::Char('k'));
    settle_until(&mut harness, |h| {
        h.is_focused(".ds-search-card input") && h.html().contains("Lunch plans")
    });
    harness.key(Key::Enter);
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    assert_eq!(
        harness.attr(".cpage .c-title input", "value").as_deref(),
        Some("Lunch plans"),
        "the page lost what was typed"
    );
}

/// A draft in Drafts opens here, and its row stays selected while it is open. A right click on
/// the row offers Delete Draft, which deletes it, closes its composer and offers it back.
#[test]
fn a_draft_opens_here_selected_and_its_menu_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let draft = mail_core::compose::draft_new(
        &store,
        account(),
        &[],
        "Offsite plans",
        "",
        chrono::Utc::now(),
    )
    .unwrap_or_else(|why| panic!("a draft: {why}"));
    let mut harness = main_window(&store);
    harness.click(centre(&harness, "[*|data-place=\"Drafts\"]"));
    let row = labelled("Open Offsite plans");
    settle_until(&mut harness, |h| h.count(&row) == 1);

    harness.click(centre(&harness, &row));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    settle_until(&mut harness, |h| {
        h.attr(&row, "aria-selected").as_deref() == Some("true")
    });

    harness.press(centre(&harness, &row), PointerButton::Secondary);
    let menu = ".ds-menu[*|aria-label=\"Actions for Offsite plans\"]";
    settle_until(&mut harness, |h| h.count(menu) == 1);
    crate::row_menu::press_menu_item(&mut harness, "Delete Draft");
    settle_until(&mut harness, |h| {
        h.count(&row) == 0 && h.count(".cpage") == 0
    });
    assert!(store.draft(draft.id).is_err(), "the draft is still stored");
    assert!(
        harness.html().contains("Draft deleted"),
        "no toast offers the draft back"
    );
}
