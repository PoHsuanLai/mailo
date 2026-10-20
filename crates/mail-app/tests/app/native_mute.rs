//! Muting a conversation, driven the way its user drives it in the real window on Blitz
//! (`ds_harness::Harness`): the reader's Mute, `m` on the open conversation and on a selection,
//! and Ctrl Z taking each back.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::Harness;

use crate::drive;
use crate::native::item;
use crate::native::ms;
use crate::opening::{Open, open_window};
use crate::store::acct_account;
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;

/// The seeded inbox, newest first, as the list draws it: (sender, subject).
const INBOX: [(&str, &str); 4] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
    ("edsger@example.test", "Notes from the review"),
];

/// A store in `dir` with one account, its identity and capabilities, and [`INBOX`].
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    seeded_with(
        dir,
        Seed {
            inbox: &INBOX,
            prefix: "mute",
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

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Click the `n`th row where a person reads it: the start of its subject line, clear of the
/// hover strip.
fn click_row(harness: &mut Harness, n: usize, held: &[Key]) {
    let subject = format!("{} .ds-thread-sub", item(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click_with(
        Point {
            x: ds::prelude::Px(rect.origin.x.0 + 24.0),
            y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
        },
        held,
    );
    harness.advance(ms(300));
}

/// Which conversations are muted, by subject, newest first, as the store has them.
fn muted(store: &SqliteStore) -> Vec<String> {
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
        .filter(|thread| thread.mute == Mute::Muted)
        .map(|thread| thread.subject)
        .collect()
}

/// Which rows (1-based) carry the muted glyph, top to bottom.
fn marked(harness: &Harness) -> Vec<usize> {
    (1..=harness.count(".list .ds-thread"))
        .filter(|n| harness.count(&format!("{} .mute-mark", item(*n))) > 0)
        .collect()
}

fn toast(harness: &Harness) -> String {
    harness.text_of(".ds-toast").unwrap_or_default()
}

/// `m` mutes the open conversation and marks its row, and Ctrl Z unmutes it. Then the reader
/// mutes the open conversation and says so, and `m` unmutes it, each its own gesture to take
/// back.
#[test]
fn m_and_the_reader_mute_and_ctrl_z_and_m_unmute() {
    let (mut harness, _dir, store) = open();
    assert_eq!(muted(&store), Vec::<String>::new(), "muted at the start");
    assert_eq!(marked(&harness), Vec::<usize>::new(), "marked at the start");

    // `m` on the open conversation. Mute is not in a row's menu: the reader's ⋯ holds it.
    click_row(&mut harness, 2, &[]);
    harness.key(Key::Char('m'));
    harness.advance(ms(600));

    assert_eq!(muted(&store), vec![INBOX[1].1.to_owned()], "`m` muted it");
    assert_eq!(marked(&harness), vec![2], "the row shows it is muted");
    assert!(
        toast(&harness).contains("Muted"),
        "`m`: {}",
        toast(&harness)
    );
    // Muting leaves the conversation where it is: it is about the replies still to come.
    assert_eq!(
        harness.count(".list .ds-thread"),
        INBOX.len(),
        "muting moved the conversation"
    );

    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "Ctrl Z unmuted it");
    assert_eq!(
        marked(&harness),
        Vec::<usize>::new(),
        "the mark outlived Ctrl Z"
    );

    // The reader: Mute is in its ⋯, and says Unmute once the conversation is muted.
    click_row(&mut harness, 3, &[]);
    let more = ".reader-head [*|aria-label=\"More\"]";
    harness.click(centre(&harness, more));
    harness.advance(ms(300));
    crate::row_menu::press_menu_item(&mut harness, "Mute");
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[2].1.to_owned()],
        "the reader's Mute"
    );
    harness.click(centre(&harness, more));
    harness.advance(ms(300));
    let names = crate::row_menu::menu_names(&harness);
    assert!(
        names.iter().any(|name| name == "Unmute"),
        "the ⋯ offers Unmute while it is muted: {names:?}"
    );
    harness.key(Key::Escape);
    harness.advance(ms(300));

    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "`m` unmuted it");

    // Each was its own gesture: Ctrl Z takes back the unmute, then the mute.
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[2].1.to_owned()],
        "Ctrl Z took back the unmute"
    );
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        Vec::<String>::new(),
        "Ctrl Z took back the mute"
    );
}

/// `m` on a selection mutes every picked conversation, and one undo takes all back. Then the
/// selection bar's Mute, on a selection half muted, mutes the rest, and its undo takes back only
/// what it did.
#[test]
fn a_selection_is_muted_as_one_gesture_by_m_and_by_the_bar() {
    let (mut harness, _dir, store) = open();

    // `m` on three picked.
    click_row(&mut harness, 1, &[]);
    click_row(&mut harness, 3, &[Key::Shift]);
    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    let three: Vec<String> = INBOX[..3].iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(muted(&store), three, "`m` on three picked");
    assert_eq!(marked(&harness), vec![1, 2, 3], "`m` on three picked");
    assert!(
        toast(&harness).contains("Muted · 3 conversations"),
        "`m` on three picked: {}",
        toast(&harness)
    );

    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "one Ctrl Z, all three");
    assert_eq!(
        marked(&harness),
        Vec::<usize>::new(),
        "the marks outlived Ctrl Z"
    );

    // The bar on a half-muted selection. The second is muted first, on its own.
    click_row(&mut harness, 2, &[]);
    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[1].1.to_owned()],
        "`m` on the second alone"
    );

    click_row(&mut harness, 1, &[]);
    click_row(&mut harness, 2, &[PRIMARY]);
    let button = "[*|aria-label=\"Mute the 2 selected\"]";
    harness.click(centre(&harness, button));
    harness.advance(ms(600));
    let two: Vec<String> = INBOX[..2].iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(muted(&store), two, "mutes the rest, never a toggle each");

    // Its undo takes back only what it did: the second stays muted.
    harness.chord(&[PRIMARY], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[1].1.to_owned()],
        "the bar's undo took back more than it did"
    );
}
