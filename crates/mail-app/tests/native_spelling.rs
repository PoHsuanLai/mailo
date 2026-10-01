//! Spelling in the composer, on the real window (Blitz, through `ds_harness::Harness`): a
//! misspelling is marked once the caret has left it, the word being typed is not, a suggestion
//! picked from the menu replaces the word as one undo step, the menu taking the keyboard neither
//! parks nor closes the draft, and turning spelling off takes the marks away.
//!
//! quire checks and draws; what is tested here is mailo's side: the setting, the caret it hands
//! the surface, and the replacement turned into one editor edit. The dictionary is a tiny one
//! written into a `TempDir` ([`Dictionaries`]), never the system's, and learned words go under
//! it too. Each case runs in a child process whose `HOME` and `XDG_DATA_HOME` are a `TempDir`, so
//! nothing it does can reach the user's own spelling list either (the workspace forbids the
//! `unsafe` that setting them in-process would need).

use ds::base::press::PointerButton;
use ds::prelude::{Point, Px, ShortcutKey as Key};
use ds::spell::lang::Lang;
use ds_blitz::spell::SpellConfig;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::harness::settle_until;
use ds_harness::{Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use mail_app::ui::native::Dictionaries;
use mail_domain::*;
use mail_store::SqliteStore;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// Set in the child process a case runs in.
const CHILD: &str = "MAILO_SPELLING_CHILD";

/// The test dictionary: UTF-8, the letters a suggestion may try, and a few words.
const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n";
const DIC: &str = "7\nthe\ncat\nsat\non\nmat\nhello\nworld\n";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// How long quire waits after typing stops before it checks.
fn debounce() -> Duration {
    ds::style::tokens::delay::DelayToken::SpellDebounce.delay()
}

/// Run `case` in a child process of this test binary with `HOME` and `XDG_DATA_HOME` in a
/// `TempDir`; in the child, run it.
fn isolated(name: &str, case: fn()) {
    if std::env::var_os(CHILD).is_some() {
        case();
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data)
        .output()
        .unwrap();
    let said = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{name} failed:\n{said}");
    assert!(said.contains("1 passed"), "{name} did not run:\n{said}");
    assert!(
        !data.join("quire").exists(),
        "the system's spelling list was written"
    );
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
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    Arc::new(store)
}

/// The test dictionary in `dir`, as the window's checker.
fn dictionaries(dir: &Path) -> Dictionaries {
    let books = dir.join("dictionaries");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::write(books.join("en_US.aff"), AFF).unwrap();
    std::fs::write(books.join("en_US.dic"), DIC).unwrap();
    Dictionaries {
        config: SpellConfig {
            dictionaries: vec![books],
            user: dir.join("learned"),
        },
        languages: vec![Lang::parse("en_US").unwrap()],
    }
}

/// The window, with a new message open and its body holding the keyboard.
fn composing() -> (Harness, tempfile::TempDir) {
    composing_in(VIEW)
}

/// [`composing`], in a window of `view`.
fn composing_in(view: Viewport) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // Every window here has a print dialog of its own: none may open the system's.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(dictionaries(dir.path()));
    let config = HarnessConfig::new(view)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    let body = centre(&harness, ".c-body");
    harness.click(body);
    harness.advance(ms(200));
    (harness, dir)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Wheel the settings list until `selector`'s centre is that element. The wheel goes through
/// the scroller's padding, at its top corner: a wheel over the look editor is consumed by the
/// editor and the list below it never moves.
fn reveal_in_scroller(harness: &mut Harness, selector: &str) {
    let scroll = harness
        .rect(".ed-scroll")
        .unwrap_or_else(|| panic!("the settings list is not drawn:\n{}", harness.html()));
    let at = Point {
        x: Px(scroll.origin.x.0 + 4.0),
        y: Px(scroll.origin.y.0 + 4.0),
    };
    for _ in 0..60 {
        if harness
            .centre(selector)
            .is_some_and(|found| harness.hits(found, selector))
        {
            return;
        }
        harness.wheel(at, Px(0.0), Px(-160.0));
        harness.advance(ms(16));
    }
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
}

fn marks(harness: &Harness) -> usize {
    harness.count(".c-body .ds-spell-mark")
}

fn body(harness: &Harness) -> String {
    harness.text_of(".c-body > p").unwrap_or_default()
}

/// Right-click the first mark, and wait for the menu to be drawn where it can be pressed.
fn open_menu(harness: &mut Harness) {
    let mark = harness
        .rect(".c-body .ds-spell-mark")
        .unwrap_or_else(|| panic!("no mark:\n{}", harness.html()));
    let on_word = Point {
        x: Px(mark.origin.x.0 + mark.size.width.0 / 2.0),
        y: Px(mark.origin.y.0 + mark.size.height.0 / 3.0),
    };
    harness.press(on_word, PointerButton::Secondary);
    settle_until(harness, |h| h.count(".ds-menu-item") > 0);
    settle_until(harness, |h| {
        h.centre(".ds-menu-item:last-child")
            .is_some_and(|at| h.hits(at, ".ds-menu-item:last-child"))
    });
}

fn a_picked_suggestion_is_one_undo_step() {
    let (mut harness, _dir) = composing();
    type_text(&mut harness, "teh cat");
    settle_until(&mut harness, |h| marks(h) == 1);
    open_menu(&mut harness);
    assert_eq!(
        harness.text_of(".ds-menu-item").as_deref().map(str::trim),
        Some("the"),
        "the best suggestion first"
    );
    // The menu has the keyboard, and the draft is still open and not put aside.
    assert!(
        !harness.is_focused(".c-body"),
        "the menu did not take the keyboard"
    );
    assert_eq!(harness.count(".cpage .c-body"), 1, "the draft closed");
    harness.click(centre(&harness, ".ds-menu-item"));
    settle_until(&mut harness, |h| body(h) == "the cat");
    settle_until(&mut harness, |h| marks(h) == 0 && h.is_focused(".c-body"));
    // One step back is the word as it was typed, and nothing before it.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    settle_until(&mut harness, |h| body(h) == "teh cat");
    // The caret is back at the end of the word it restored, so the word waits for it to leave.
    harness.key(Key::End);
    settle_until(&mut harness, |h| marks(h) == 1);
    // And the keyboard is the body's: typing goes on at the caret.
    type_text(&mut harness, " sat");
    settle_until(&mut harness, |h| body(h) == "teh cat sat");
}

#[test]
fn a_picked_suggestion_replaces_the_word_and_one_ctrl_z_restores_it() {
    isolated(
        "a_picked_suggestion_replaces_the_word_and_one_ctrl_z_restores_it",
        a_picked_suggestion_is_one_undo_step,
    );
}

fn the_word_at_the_caret_waits() {
    let (mut harness, _dir) = composing();
    type_text(&mut harness, "hello teh");
    harness.advance(debounce() * 3);
    assert_eq!(marks(&harness), 0, "the word being typed was marked");
    type_text(&mut harness, " ");
    settle_until(&mut harness, |h| marks(h) == 1);
    // The mark is under the second word, on the paragraph's line, not wherever its layer sits.
    let mark = harness.rect(".c-body .ds-spell-mark").unwrap();
    let line = harness.rect(".c-body > p").unwrap();
    let left = mark.origin.x.0 - line.origin.x.0;
    assert!((25.0..60.0).contains(&left), "the mark starts at {left}");
    let bottom = mark.origin.y.0 + mark.size.height.0 - line.origin.y.0;
    assert!(
        (line.size.height.0 * 0.5..=line.size.height.0 + 2.0).contains(&bottom),
        "the dots sit {bottom} below a line {} tall",
        line.size.height.0
    );
}

#[test]
fn the_word_under_the_caret_is_not_marked_until_the_caret_leaves_it() {
    isolated(
        "the_word_under_the_caret_is_not_marked_until_the_caret_leaves_it",
        the_word_at_the_caret_waits,
    );
}

fn escape_keeps_the_draft() {
    let (mut harness, _dir) = composing();
    type_text(&mut harness, "teh cat");
    settle_until(&mut harness, |h| marks(h) == 1);
    open_menu(&mut harness);
    harness.key(Key::Escape);
    settle_until(&mut harness, |h| h.count(".ds-menu-item") == 0);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".cpage .c-body"),
        1,
        "Escape in the menu put the draft aside"
    );
    settle_until(&mut harness, |h| h.is_focused(".c-body"));
    type_text(&mut harness, "s");
    settle_until(&mut harness, |h| body(h) == "teh cats");
}

#[test]
fn escape_closes_the_spelling_menu_and_leaves_the_draft_open() {
    isolated(
        "escape_closes_the_spelling_menu_and_leaves_the_draft_open",
        escape_keeps_the_draft,
    );
}

/// [`settle_until`], naming the state it was waiting on.
fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let started = harness.now();
    while harness.now().saturating_duration_since(started) < Duration::from_secs(3) {
        if done(harness) {
            return;
        }
        harness.advance(Duration::from_millis(10));
    }
    assert!(done(harness), "{what}:\n{}", harness.html());
}

fn off_removes_the_marks() {
    let (mut harness, _dir) = composing_in(VIEW);
    type_text(&mut harness, "teh cat");
    settle_until(&mut harness, |h| marks(h) == 1);
    // The settings, from the Space's name in the sidebar, beside the open draft.
    harness.click(centre(&harness, ".space-name"));
    // Blitz's selectors name an attribute with a dash through the any-namespace form. Off is the
    // second segment; the thumb is the control's last child, so it is not `button:last-child`.
    let off = "[*|aria-label=\"Check spelling\"] .ds-segmented-segment:nth-child(2)";
    until(&mut harness, "Off is drawn", |h| {
        h.text_of(off).is_some_and(|text| text.trim() == "Off")
    });
    // A wheel over the look editor is consumed there. The scroller's own padding is the
    // settings list, and that is what has to move for Off to come into reach.
    reveal_in_scroller(&mut harness, off);
    until(&mut harness, "Off can be pressed", |h| {
        h.centre(off).is_some_and(|at| h.hits(at, off))
    });
    harness.click(centre(&harness, off));
    until(&mut harness, "the marks leave", |h| marks(h) == 0);
    harness.advance(debounce() * 3);
    assert_eq!(marks(&harness), 0, "a mark came back with spelling off");
    assert_eq!(body(&harness), "teh cat", "the draft changed");
}

#[test]
fn turning_spelling_off_removes_the_marks() {
    isolated(
        "turning_spelling_off_removes_the_marks",
        off_removes_the_marks,
    );
}
