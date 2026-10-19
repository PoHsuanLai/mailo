//! Emoji in the composer, on the real window (Blitz, through `ds_harness::Harness`): `:smi` offers
//! emoji by name and Enter puts the first in the body, where one Ctrl Z takes it out again; `/` ›
//! Emoji… opens the picker at the caret (the foot has no emoji button), where a click picks; the
//! picker is walked with the arrows, Enter and Escape; and what was picked is first in a new
//! window's picker.
//!
//! The recent emoji are kept in the window's state directory, here a `TempDir`.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Clock, Harness};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::native::ms;
use crate::opening::{Open, Wait, open_window};
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_app::ui::appearance::WindowDirs;
use mail_core::SqliteStore;
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
    assert_eq!(
        harness.count(".c-foot [*|aria-label=\"Emoji\"]"),
        0,
        "the foot has an emoji button"
    );
    type_text(harness, "/emoj");
    settle_until(harness, |h| {
        h.text_of(".ds-menu-item")
            .is_some_and(|row| row.contains("Emoji"))
    });
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
fn a_colon_and_a_name_offer_emoji_enter_puts_one_in_and_one_ctrl_z_takes_it_out() {
    let (mut harness, _dir, _store, _dirs) = composing();
    type_text(&mut harness, ":smi");
    settle_until(&mut harness, |h| h.count(".ds-menu-item") > 0);
    assert_eq!(
        harness
            .text_of(".ds-menu-item")
            .as_deref()
            .map(str::trim)
            .map(|text| text.contains("smiling face with smiling eyes")),
        Some(true),
        "the first row is the first name that starts with it:\n{}",
        harness.html()
    );
    assert!(harness.is_focused(".c-body"), "the body kept the keyboard");
    harness.key(Key::Enter);
    settle_until(&mut harness, |h| body(h) == "😊");
    settle_until(&mut harness, |h| h.count(".ds-menu-item") == 0);
    assert_eq!(harness.count(".c-body > p"), 1, "Enter made no new line");

    harness.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut harness, |h| body(h) == ":smi");
    // The keyboard is still the body's. (An undo leaves the caret where it was, clamped: the
    // editor's undo says nothing about the caret, so End takes it past the name again.)
    harness.key(Key::End);
    type_text(&mut harness, "le");
    settle_until(&mut harness, |h| body(h) == ":smile");
}

#[test]
fn slash_emoji_opens_the_picker_a_click_picks_and_a_new_window_has_it_first() {
    let (mut harness, _dir, store, dirs) = composing();
    type_text(&mut harness, "Hi ");
    settle_until(&mut harness, |h| body(h).trim_end() == "Hi");
    open_picker(&mut harness);
    // Nothing picked yet: the first group.
    assert_eq!(first_cell(&harness).as_deref(), Some("grinning face"));
    harness.click(centre(&harness, CELL));
    settle_until(&mut harness, |h| h.count(".em-picker") == 0);
    settle_until(&mut harness, |h| body(h) == "Hi 😀");
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
    type_text(&mut harness, "!");
    settle_until(&mut harness, |h| body(h) == "Hi 😀!");
    assert!(
        dirs.state
            .join(mail_app::ui::emoji::recent::FILE_NAME)
            .exists(),
        "the pick was kept"
    );

    // A second window over the same directories has it first, under the recent tab.
    drop(harness);
    let mut harness = composing_over(&store, &dirs);
    open_picker(&mut harness);
    assert_eq!(first_cell(&harness).as_deref(), Some("grinning face"));
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

#[test]
fn the_picker_is_walked_with_the_arrows_enter_picks_and_escape_closes_it() {
    let (mut harness, _dir, _store, _dirs) = composing();
    open_picker(&mut harness);
    type_text(&mut harness, "cat");
    let found: Vec<&str> = mail_app::ui::emoji::search("cat")
        .iter()
        .map(|emoji| emoji.name)
        .collect();
    settle_until(&mut harness, |h| {
        first_cell(h).as_deref() == found.first().copied()
    });
    harness.key(Key::Right);
    settle_until(&mut harness, |h| {
        h.text_of(".em-name").as_deref().map(str::trim) == found.get(1).copied()
    });
    harness.key(Key::Enter);
    settle_until(&mut harness, |h| h.count(".em-picker") == 0);
    let second = mail_app::ui::emoji::search("cat")[1].glyph;
    settle_until(&mut harness, |h| body(h) == second);

    // Escape closes the picker and nothing else: the draft stays open, the body unchanged. A `/`
    // opens its menu at the start of a word, so a space goes first.
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
    type_text(&mut harness, " ");
    open_picker(&mut harness);
    harness.key(Key::Escape);
    settle_until(&mut harness, |h| h.count(".em-picker") == 0);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".cpage .c-body"),
        1,
        "Escape put the draft aside"
    );
    assert_eq!(body(&harness).trim_end(), second);
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
}
