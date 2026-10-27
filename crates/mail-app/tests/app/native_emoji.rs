//! Emoji in the composer, on the real window (Blitz, through `ds_harness::Harness`): `:smi` offers
//! emoji by name and Enter puts the first in the body, where one Ctrl Z takes it out again; `/` ›
//! Emoji… opens the picker at the caret (the foot has no emoji button), where a click picks; and
//! what was picked is first in a new window's picker.
//!
//! The recent emoji are kept in the window's state directory, here a `TempDir`.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Clock, Harness};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::opening::{Open, Wait, open_window};
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_core::SqliteStore;
use mail_ui::ui::appearance::WindowDirs;
use std::sync::Arc;

/// A cell of the picker's grid.
const CELL: &str = ".em-picker .ds-emoji-cell";

/// One account and its identity: enough for `c` to open a new message.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    seeded_with(
        dir,
        Seed {
            caps: None,
            ..Seed::default()
        },
    )
}

/// A window over `store` and `dirs`, with a new message open and its body holding the keyboard.
fn composing_over(store: &Arc<SqliteStore>, dirs: &WindowDirs) -> Harness {
    let mut harness = open_window(
        store,
        Open {
            dirs: Some(dirs.clone()),
            clock: Clock::Wall,
            wait: Wait::Entrances,
            ..Open::default()
        },
    );
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    let body = centre(&harness, ".c-body");
    harness.click(body);
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
    harness
}

/// The window, over a fresh store and fresh directories. The `TempDir` must outlive the rest.
fn composing() -> (Harness, tempfile::TempDir, Arc<SqliteStore>, WindowDirs) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let harness = composing_over(&store, &dirs);
    (harness, dir, store, dirs)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
}

fn body(harness: &Harness) -> String {
    harness.text_of(".c-body > p").unwrap_or_default()
}

/// Type `/emoj` in the body, which leaves Emoji… the one row (`/emo` also finds Code), and press
/// Enter on it, and wait for the picker's cells to be drawn
/// where they can be pressed, with its search field holding the keyboard.
fn open_picker(harness: &mut Harness) {
    type_text(harness, "/emoj");
    settle_until(harness, |h| h.count(".ds-menu-item") == 1);
    harness.key(Key::Enter);
    settle_until(harness, |h| {
        h.centre(CELL).is_some_and(|at| h.hits(at, CELL)) && h.is_focused(".em-picker input")
    });
}

/// The name of the picker's first cell.
fn first_cell(harness: &Harness) -> Option<String> {
    harness.attr(CELL, "aria-label")
}

#[test]
fn a_name_after_a_colon_and_the_picker_put_emoji_in_and_a_new_window_has_the_pick_first() {
    let (mut harness, _dir, store, dirs) = composing();

    // `:smi` offers by name with the body keeping the keyboard; Enter puts the first row in,
    // making no new line, and one undo takes it out again.
    type_text(&mut harness, ":smi");
    settle_until(&mut harness, |h| h.count(".ds-menu-item") > 0);
    assert!(harness.is_focused(".c-body"), "the body kept the keyboard");
    harness.key(Key::Enter);
    settle_until(&mut harness, |h| body(h) == "😊");
    settle_until(&mut harness, |h| h.count(".ds-menu-item") == 0);
    assert_eq!(harness.count(".c-body > p"), 1, "Enter made no new line");
    harness.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut harness, |h| body(h) == ":smi");
    // An undo says nothing about the caret, so End takes it past the name again.
    harness.key(Key::End);
    type_text(&mut harness, "le ");
    settle_until(&mut harness, |h| body(h).trim_end() == ":smile");

    // `/emoj` › Emoji…: the emoji the name put in is already recent, first in the grid; a click
    // picks it again and gives the body the keyboard back.
    open_picker(&mut harness);
    let first = first_cell(&harness).expect("the first cell is named");
    harness.click(centre(&harness, CELL));
    settle_until(&mut harness, |h| h.count(".em-picker") == 0);
    settle_until(&mut harness, |h| body(h) == ":smile 😊");
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
    type_text(&mut harness, "!");
    settle_until(&mut harness, |h| body(h) == ":smile 😊!");
    assert!(
        dirs.state
            .join(mail_ui::ui::emoji::recent::FILE_NAME)
            .exists(),
        "the pick was kept"
    );

    // A second window over the same directories opens on the recent tab, holding only that one.
    drop(harness);
    let mut harness = composing_over(&store, &dirs);
    open_picker(&mut harness);
    assert_eq!(first_cell(&harness), Some(first));
    assert_eq!(harness.count(CELL), 1, "only what was picked is recent");
    assert_eq!(
        harness
            .text_of(".em-tabs [*|aria-checked=true]")
            .as_deref()
            .map(str::trim),
        Some("🕘"),
        "the recent tab is the one open"
    );
}
