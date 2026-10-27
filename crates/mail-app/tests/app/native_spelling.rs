//! Spelling in the composer, on the real window (Blitz, through `ds_harness::Harness`): a
//! misspelling is marked once the caret has left it, the word being typed is not, the menu
//! taking the keyboard neither parks nor closes the draft, a suggestion picked from it replaces
//! the word as one undo step, and turning spelling off takes the marks away.
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
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::native::ms;
use crate::store::{Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_app::ui::native::{Configured, Dictionaries};

/// The two counters every window of the app shares: the store's, and the configuration files'.
use mail_core::SqliteStore;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The Settings window at the size it opens at.
const SETTINGS_VIEW: Viewport = Viewport {
    width: 780,
    height: 620,
    scale_percent: 100,
};

/// Set in the child process a case runs in.
const CHILD: &str = "MAILO_SPELLING_CHILD";

/// The test dictionary: UTF-8, the letters a suggestion may try, and a few words.
const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n";
const DIC: &str = "7\nthe\ncat\nsat\non\nmat\nhello\nworld\n";

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
    // The test's full name in this binary: its module path without the crate's own name.
    let full = match module_path!().split_once("::") {
        Some((_, module)) => format!("{module}::{name}"),
        None => name.to_owned(),
    };
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([full.as_str(), "--exact", "--nocapture", "--test-threads=1"])
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
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    seeded_with(
        dir,
        Seed {
            caps: None,
            ..Seed::default()
        },
    )
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

/// What a window here is given: the store, scratch directories, the test's dictionaries, and
/// the configuration count every window of the app shares.
fn contexts(
    dir: &std::path::Path,
    store: &Arc<SqliteStore>,
    configured: &Configured,
) -> RootContexts {
    // Every window here has a print dialog of its own: none may open the system's.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        Some(mail_app::ui::appearance::WindowDirs {
            config: dir.join("config"),
            state: dir.join("state"),
        }),
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(dictionaries(dir))
    .with(configured.clone())
}

/// The main window with a new message open and its body focused.
struct Composing {
    harness: Harness,
    dir: tempfile::TempDir,
    store: Arc<SqliteStore>,
    configured: Configured,
}

fn composing_beside(view: Viewport) -> Composing {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let configured = Configured::default();
    let config = HarnessConfig::new(view)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(dir.path(), &store, &configured));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list-title") > 0);
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    let body = centre(&harness, ".c-body");
    harness.click(body);
    harness.advance(ms(200));
    Composing {
        harness,
        dir,
        store,
        configured,
    }
}

/// The Settings window beside `open`'s main window: the same store, directories and
/// configuration count.
fn settings_beside(open: &Composing) -> Harness {
    let config = HarnessConfig::new(SETTINGS_VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(open.dir.path(), &open.store, &open.configured));
    let mut harness = Harness::new(mail_app::ui::native::settings_root, config);
    settle_until(&mut harness, |h| h.count(".settings-page") == 1);
    harness
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

/// One draft, start to end: the word at the caret waits, the mark lands under the word that
/// was left, Escape closes the menu and keeps the draft, a pick is one undo step, and spelling
/// turned off in Settings takes the marks away.
fn a_misspelling_is_marked_mended_and_unmarked() {
    let mut open = composing_beside(VIEW);
    let harness = &mut open.harness;
    type_text(harness, "hello teh");
    harness.advance(debounce() * 3);
    assert_eq!(marks(harness), 0, "the word being typed was marked");
    type_text(harness, " cat");
    settle_until(harness, |h| marks(h) == 1);
    // The mark is under the second word, on the paragraph's line, not wherever its layer sits.
    let mark = harness.rect(".c-body .ds-spell-mark").unwrap();
    let line = harness.rect(".c-body > p").unwrap();
    let left = mark.origin.x.0 - line.origin.x.0;
    assert!((25.0..60.0).contains(&left), "the mark starts at {left}");

    // The menu takes the keyboard; Escape closes it and leaves the draft open.
    open_menu(harness);
    assert!(
        !harness.is_focused(".c-body"),
        "the menu did not take the keyboard"
    );
    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(".ds-menu-item") == 0);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".cpage .c-body"),
        1,
        "Escape put the draft aside"
    );

    // The best suggestion first; picked, it replaces the word, and one step back restores it.
    open_menu(harness);
    assert_eq!(
        harness.text_of(".ds-menu-item").as_deref().map(str::trim),
        Some("the")
    );
    harness.click(centre(harness, ".ds-menu-item"));
    settle_until(harness, |h| body(h) == "hello the cat");
    settle_until(harness, |h| marks(h) == 0 && h.is_focused(".c-body"));
    harness.chord(&[PRIMARY], Key::Char('z'));
    settle_until(harness, |h| body(h) == "hello teh cat");
    harness.key(Key::End);
    settle_until(harness, |h| marks(h) == 1);

    // Settings, a window of its own beside the draft: the switch off, and the marks leave.
    let mut settings = settings_beside(&open);
    let off = "[*|role=switch][*|aria-label=\"Check spelling\"]";
    until(&mut settings, "the switch is drawn, on", |h| {
        h.attr(off, "aria-checked").as_deref() == Some("true")
    });
    let at = centre(&settings, off);
    settings.click(at);
    until(&mut settings, "the switch is off", |h| {
        h.attr(off, "aria-checked").as_deref() == Some("false")
    });
    let harness = &mut open.harness;
    until(harness, "the marks leave", |h| marks(h) == 0);
    harness.advance(debounce() * 3);
    assert_eq!(marks(harness), 0, "a mark came back with spelling off");
    assert_eq!(body(harness), "hello teh cat", "the draft changed");
}

#[test]
fn a_misspelling_is_marked_once_left_mended_from_its_menu_and_unmarked_when_spelling_is_off() {
    isolated(
        "a_misspelling_is_marked_once_left_mended_from_its_menu_and_unmarked_when_spelling_is_off",
        a_misspelling_is_marked_mended_and_unmarked,
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
