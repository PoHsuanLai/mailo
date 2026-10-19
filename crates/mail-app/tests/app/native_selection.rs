//! Several conversations picked at once, driven the way their user drives them: Shift-click,
//! Ctrl-click, Shift+j/k and Ctrl A in the real window on Blitz (`ds_harness::Harness`), and an
//! action on the selection taken back by one Ctrl Z.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::Harness;

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::native::ms;
use crate::native::row;
use crate::opening::{Open, open_window};
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;

/// The seeded inbox, newest first, as the list draws it: (sender, subject).
const INBOX: [(&str, &str); 5] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
    ("edsger@example.test", "Notes from the review"),
    ("barbara@example.test", "The shared drive is full"),
];

/// A store in `dir` with one IMAP-like account (archive drops the inbox, as on a label server)
/// and [`INBOX`].
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    seeded_with(
        dir,
        Seed {
            inbox: &INBOX,
            prefix: "pick",
            ..Seed::default()
        },
    )
}

/// The window over a freshly seeded store, first frame drawn, with the store it writes. The
/// `TempDir` must outlive both.
fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let harness = open_window(&store, Open::default());
    (harness, dir, store)
}

/// Where a person clicks the `n`th row: the start of its subject line, clear of the hover strip.
fn subject_of(harness: &Harness, n: usize) -> Point {
    let subject = format!("{} .ds-thread-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    }
}

fn click_row(harness: &mut Harness, n: usize, held: &[Key]) {
    let at = subject_of(harness, n);
    harness.click_with(at, held);
    harness.advance(ms(300));
}

/// Which rows (1-based) are drawn selected, top to bottom.
fn selected(harness: &Harness) -> Vec<usize> {
    (1..=harness.count(".list .ds-thread"))
        .filter(|n| harness.attr(&row(*n), "aria-selected").as_deref() == Some("true"))
        .collect()
}

/// The subjects of the list's rows, top to bottom.
fn subjects(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".list .ds-thread"))
        .filter_map(|n| harness.text_of(&format!("{} .ds-thread-sub", row(n))))
        .collect()
}

/// What the list bar says about the selection, if anything.
fn said(harness: &Harness) -> Option<String> {
    let html = harness.html();
    let before = &html[..html.find(" selected</span>")?];
    let count: String = before
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(format!("{count} selected"))
}

fn inbox_count(store: &SqliteStore) -> usize {
    let query = Query {
        filter: Filter::InMailbox(MailboxRole::Inbox),
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
        .len()
}

/// The frame of the open conversation holds `body`.
fn reader_shows(harness: &Harness, body: &str) -> bool {
    harness
        .frame("article.frame iframe.html")
        .is_some_and(|frame| frame.html().contains(body))
}

/// Rows are picked by Ctrl+A, Shift-click, Ctrl-click and Shift+J/K, each step starting from what
/// the one before left: Ctrl+A picks every row and Escape lets them go; the selection does not
/// follow to another place; Shift-click picks the range from the open row; Ctrl-click adds and
/// takes away one row at a time and does not open it; Shift+J and Shift+K move the end of the
/// range and open nothing on the way.
#[test]
fn rows_are_picked_by_ctrl_a_shift_and_ctrl_clicks_and_shift_j_and_k() {
    let (mut harness, _dir, _store) = open();

    // Ctrl+A, then Escape.
    assert_eq!(
        selected(&harness),
        Vec::<usize>::new(),
        "picked at the start"
    );
    harness.chord(&[PRIMARY], Key::Char('a'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![1, 2, 3, 4, 5], "Ctrl+A");
    assert_eq!(said(&harness).as_deref(), Some("5 selected"), "Ctrl+A");
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(selected(&harness), Vec::<usize>::new(), "Escape");
    assert_eq!(said(&harness), None, "Escape");

    // Ctrl+A, then to Starred and back: the selection does not follow.
    harness.chord(&[PRIMARY], Key::Char('a'));
    harness.advance(ms(300));
    assert_eq!(
        said(&harness).as_deref(),
        Some("5 selected"),
        "Ctrl+A again"
    );
    let place = |name: &str| format!("[*|data-place=\"{name}\"]");
    let at = harness
        .centre(&place("Starred"))
        .unwrap_or_else(|| panic!("no Starred place:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(600));
    let at = harness.centre(&place("Inbox")).expect("the Inbox place");
    harness.click(at);
    harness.advance(ms(600));
    assert_eq!(subjects(&harness).len(), INBOX.len(), "back in the inbox");
    assert_eq!(
        selected(&harness),
        Vec::<usize>::new(),
        "the selection followed to another place"
    );
    assert_eq!(
        said(&harness),
        None,
        "the selection followed to another place"
    );

    // Shift-click.
    click_row(&mut harness, 2, &[]);
    assert_eq!(selected(&harness), vec![2], "a plain click selects its row");
    assert_eq!(said(&harness), None, "one open row is not a selection");
    click_row(&mut harness, 4, &[Key::Shift]);
    assert_eq!(selected(&harness), vec![2, 3, 4], "Shift-click down");
    assert_eq!(
        said(&harness).as_deref(),
        Some("3 selected"),
        "Shift-click down"
    );
    // Measured again from the same anchor, upward this time.
    click_row(&mut harness, 1, &[Key::Shift]);
    assert_eq!(selected(&harness), vec![1, 2], "Shift-click up");
    // A plain click lets the selection go.
    click_row(&mut harness, 5, &[]);
    assert_eq!(
        selected(&harness),
        vec![5],
        "a plain click after Shift-click"
    );
    assert_eq!(said(&harness), None, "a plain click after Shift-click");

    // Ctrl-click.
    click_row(&mut harness, 1, &[]);
    click_row(&mut harness, 3, &[PRIMARY]);
    assert_eq!(
        selected(&harness),
        vec![1, 3],
        "the open row and the added one"
    );
    click_row(&mut harness, 5, &[PRIMARY]);
    assert_eq!(
        selected(&harness),
        vec![1, 3, 5],
        "a second Ctrl-click adds"
    );
    click_row(&mut harness, 1, &[PRIMARY]);
    assert_eq!(
        selected(&harness),
        vec![3, 5],
        "a second Ctrl-click takes it out"
    );
    assert_eq!(said(&harness).as_deref(), Some("2 selected"), "Ctrl-click");
    // The reader still shows what was opened: Ctrl-click picks, it does not open.
    settle_until(&mut harness, |h| {
        reader_shows(h, "The body of Flight to the conference.")
    });

    // Shift+J and Shift+K.
    click_row(&mut harness, 2, &[]);
    harness.chord(&[Key::Shift], Key::Char('j'));
    harness.chord(&[Key::Shift], Key::Char('j'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![2, 3, 4], "Shift+J twice");
    harness.chord(&[Key::Shift], Key::Char('k'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![2, 3], "Shift+K");
    // Nothing was opened on the way: the reader is on the row that was clicked.
    settle_until(&mut harness, |h| {
        reader_shows(h, "The body of The invoice for September.")
    });
}

#[test]
fn archiving_a_selection_is_one_gesture_and_one_undo_puts_it_all_back() {
    let (mut harness, _dir, store) = open();
    let before = inbox_count(&store);
    assert_eq!(before, INBOX.len());
    click_row(&mut harness, 2, &[]);
    click_row(&mut harness, 4, &[Key::Shift]);
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    assert_eq!(
        inbox_count(&store),
        before - 3,
        "the archive did not reach all three"
    );
    assert_eq!(
        subjects(&harness),
        vec![
            "Flight to the conference".to_owned(),
            "The shared drive is full".to_owned(),
        ]
    );
    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Archived · 3 conversations"), "{toast}");
    // One Ctrl Z: every one of the three is back, not only the last.
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(1500));
    assert_eq!(inbox_count(&store), before, "the undo left some archived");
    let want: Vec<String> = INBOX.iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(subjects(&harness), want);
    // And that was the whole entry: a second Ctrl Z has nothing of this left to take back.
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(inbox_count(&store), before);
}

#[test]
fn a_button_on_the_selection_bar_acts_on_every_picked_row() {
    let (mut harness, _dir, store) = open();
    click_row(&mut harness, 1, &[]);
    click_row(&mut harness, 3, &[PRIMARY]);
    let star = "[*|aria-label=\"Star the 2 selected\"]";
    let at = harness
        .centre(star)
        .unwrap_or_else(|| panic!("no Star on the selection bar:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(600));
    let starred: Vec<String> = subjects_with(&store, Star::Starred);
    assert_eq!(
        starred,
        vec![
            "Flight to the conference".to_owned(),
            "Lunch on Thursday".to_owned(),
        ]
    );
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(subjects_with(&store, Star::Starred), Vec::<String>::new());
}

/// The subjects of the inbox's conversations with `star`, newest first.
fn subjects_with(store: &SqliteStore, star: Star) -> Vec<String> {
    let query = Query {
        filter: Filter::InMailbox(MailboxRole::Inbox),
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
        .filter(|thread| thread.star == star)
        .map(|thread| thread.subject)
        .collect()
}

/// Every button of the selection's tools, as a selector for it alone. While rows are picked the
/// list head is that bar, and its buttons are the tools' own children.
fn bar_buttons(harness: &Harness) -> Vec<String> {
    let n = harness.count(".pick-tools > button");
    let found: Vec<String> = (1..=n)
        .map(|k| format!(".pick-tools > button:nth-child({k})"))
        .filter(|selector| harness.count(selector) == 1)
        .collect();
    assert_eq!(
        found.len(),
        n,
        "a bar button was not enumerated one by one:\n{}",
        harness.html()
    );
    found
}

/// At the window's usual width, with two rows picked, every button in the list bar lies inside
/// the list column, and a press there reaches it rather than the reader beside it. The old bar
/// kept the page's tools beside the selection's, and they ran to x≈950 over a column that ends
/// at ≈693.
#[test]
fn with_rows_picked_every_bar_button_is_inside_the_list_column() {
    let (mut harness, _dir, _store) = open();
    click_row(&mut harness, 1, &[]);
    click_row(&mut harness, 3, &[PRIMARY]);
    assert_eq!(said(&harness).as_deref(), Some("2 selected"));
    let column = harness.rect(".list-col").expect("the list column");
    let (left, right) = (column.origin.x.0, column.origin.x.0 + column.size.width.0);
    let (top, bottom) = (column.origin.y.0, column.origin.y.0 + column.size.height.0);
    let buttons = bar_buttons(&harness);
    assert!(
        buttons.len() >= 5,
        "the selection bar has too few buttons: {buttons:?}"
    );
    for button in &buttons {
        let rect = harness.rect(button).expect("an enumerated button");
        let (x0, x1) = (rect.origin.x.0, rect.origin.x.0 + rect.size.width.0);
        let (y0, y1) = (rect.origin.y.0, rect.origin.y.0 + rect.size.height.0);
        assert!(
            x0 >= left && x1 <= right && y0 >= top && y1 <= bottom,
            "{button} ({x0}..{x1}, {y0}..{y1}) is outside the list column \
             ({left}..{right}, {top}..{bottom})"
        );
        let centre = harness.centre(button).expect("an enumerated button");
        assert!(
            harness.hits(centre, button),
            "a press on {button}'s centre does not reach it"
        );
    }
    // And a real press on the last of them, the one furthest right, acts: it clears the pick.
    let last = buttons.last().expect("at least one button");
    assert_eq!(
        harness.attr(last, "aria-label").as_deref(),
        Some("Clear the selection"),
        "the bar's last button is not Clear"
    );
    harness.click(harness.centre(last).expect("Clear"));
    harness.advance(ms(300));
    assert_eq!(said(&harness), None, "the press on Clear did not reach it");
    assert_eq!(
        selected(&harness),
        vec![1],
        "only the open row is drawn selected"
    );
}
