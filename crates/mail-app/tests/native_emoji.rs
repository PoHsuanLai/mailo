//! Emoji in the composer, on the real window (Blitz, through `ds_harness::Harness`): `:smi` offers
//! emoji by name and Enter puts the first in the body, where one Ctrl Z takes it out again; the
//! foot's button opens the picker, where a click picks; the picker is walked with the arrows,
//! Enter and Escape; and what was picked is first in a new window's picker.
//!
//! The recent emoji are kept in the window's state directory, here a `TempDir`.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use mail_app::ui::appearance::WindowDirs;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The foot's button. Blitz's selectors name an attribute with a dash through the any-namespace
/// form.
const BUTTON: &str = ".c-foot [*|aria-label=\"Emoji\"]";

/// A cell of the picker's grid.
const CELL: &str = ".em-picker .ds-emoji-cell";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// One account and its identity: enough for `c` to open a new message.
fn seeded(dir: &Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [
                IdentityId::generate().to_string(),
                acct_account().to_string(),
            ],
        )
        .unwrap();
    }
    Arc::new(store)
}

/// A window over `store` and `dirs`, with a new message open and its body holding the keyboard.
fn composing_over(store: &Arc<SqliteStore>, dirs: &WindowDirs) -> Harness {
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        Some(dirs.clone()),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        // Wall: on the virtual clock the picker opened a second time never takes the keyboard
        // into its search field. Waits are `settle_until`, bounded by the machine's time.
        .with_clock(Clock::Wall)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
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

/// Press the foot's button and wait for the picker's cells to be drawn where they can be
/// pressed, with its search field holding the keyboard.
fn open_picker(harness: &mut Harness) {
    settle_until(harness, |h| {
        h.centre(BUTTON).is_some_and(|at| h.hits(at, BUTTON))
    });
    harness.click(centre(harness, BUTTON));
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

    harness.chord(&[Key::Ctrl], Key::Char('z'));
    settle_until(&mut harness, |h| body(h) == ":smi");
    // The keyboard is still the body's. (An undo leaves the caret where it was, clamped: the
    // editor's undo says nothing about the caret, so End takes it past the name again.)
    harness.key(Key::End);
    type_text(&mut harness, "le");
    settle_until(&mut harness, |h| body(h) == ":smile");
}

#[test]
fn the_button_opens_the_picker_a_click_picks_and_a_new_window_has_it_first() {
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

    // Escape closes the picker and nothing else: the draft stays open, the body unchanged.
    open_picker(&mut harness);
    harness.key(Key::Escape);
    settle_until(&mut harness, |h| h.count(".em-picker") == 0);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".cpage .c-body"),
        1,
        "Escape put the draft aside"
    );
    assert_eq!(body(&harness), second);
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
}
