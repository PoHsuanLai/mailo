//! Custom keyboard shortcuts, driven the way their user drives them in the real window on Blitz
//! (`ds_harness::Harness`): the Settings window's Keyboard page, Change on Archive, a key that
//! another action holds refused by that action's name, a free key taken, and then, in the main
//! window, opened afterwards as mailo reads the keys when it opens, the new key archiving and the
//! old one doing nothing.
//!
//! The window is handed a pair of `TempDir` directories, so the keymap it keeps lands there and
//! nowhere else; the store is seeded in a `TempDir` too.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use drive::Drive;
use mail_app::ui::appearance::WindowDirs;
use mail_app::ui::native::{Configured, Revisions};

/// The two counters every window of the app shares: the store's, and the configuration files'.
type Shared = (Revisions, Configured);
use mail_app::ui::view::Shortcut;
use mail_core::{Arrival, absorb};
use mail_core::{SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b7"))
}

/// The main window, as a laptop gives it.
const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The Settings window at the size it opens at: the Keyboard page's last row, Reset All, is below
/// the fold and is reached by scrolling.
const SETTINGS_VIEW: Viewport = Viewport {
    width: 780,
    height: 620,
    scale_percent: 100,
};

const INBOX: [(&str, &str); 3] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A store in `dir` with one account, its identity and capabilities, and [`INBOX`].
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
        let caps = AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: FolderRoles::default(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Yes,
            connections: ConnectionBudget::default(),
            observed_at: chrono::Utc::now(),
        };
        mail_store::testing::seed_caps(&store, acct_account(), &caps, chrono::Utc::now()).unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <keys{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
        );
        absorb(
            &store,
            acct_account(),
            MailboxRef {
                account: acct_account(),
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("keys{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// What both windows are given: the store, the directories, and one revision between them,
/// as quire hands every window of the app the same root contexts.
fn contexts(
    store: &Arc<SqliteStore>,
    dirs: &WindowDirs,
    revisions: &Shared,
) -> ds_blitz::RootContexts {
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        Some(dirs.clone()),
        mail_app::ui::Start::Inbox,
    )
    // The keys kept in the directory, as the launch lays them over the keymap as mailo opens.
    .with(
        ds::prelude::KeySource::default()
            .with_overrides(mail_app::ui::keymap::load(&dirs.config).overrides()),
    )
    .with(printer)
    .with(revisions.0.clone())
    .with(revisions.1.clone())
}

/// The main window, first frame drawn.
fn main_window(store: &Arc<SqliteStore>, dirs: &WindowDirs, revisions: &Shared) -> Harness {
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(store, dirs, revisions));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    settle_until(&mut harness, |harness| {
        harness.count(".list .ds-thread") == INBOX.len()
    });
    harness
}

/// The Settings window, at the size it opens at, first frame drawn.
fn settings_window(store: &Arc<SqliteStore>, dirs: &WindowDirs, revisions: &Shared) -> Harness {
    let config = HarnessConfig::new(SETTINGS_VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(store, dirs, revisions));
    let mut harness = Harness::new(mail_app::ui::native::settings_root, config);
    settle_until(&mut harness, |harness| harness.count(GENERAL) == 1);
    harness
}

/// Both windows over a freshly seeded store and a pair of empty directories. The `TempDir` must
/// outlive the rest.
fn open() -> (
    Harness,
    Harness,
    tempfile::TempDir,
    Arc<SqliteStore>,
    WindowDirs,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let revisions = (Revisions::new(), Configured::default());
    let main = main_window(&store, &dirs, &revisions);
    let settings = settings_window(&store, &dirs, &revisions);
    (main, settings, dir, store, dirs)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn click(harness: &mut Harness, selector: &str) {
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(300));
}

/// Conversations in the inbox, as the store has them.
fn in_inbox(store: &SqliteStore) -> usize {
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

const PAGE: &str = ".settings-page[*|data-page=\"Keyboard\"]";
/// The page the window opens on.
const GENERAL: &str = ".settings-page[*|data-page=\"General\"]";
const CHANGE_ARCHIVE: &str = "[*|aria-label=\"Change the key for Archive\"]";
/// What Archive's row says under its name: why its last key was refused.
const ARCHIVE_SAID: &str = "[*|data-action=Archive] .ds-field-row-help";

const SCROLLER: &str = ".settings-scroll";
const SIDEBAR_KEYBOARD: &str = ".ds-sidebar [*|aria-label=\"Keyboard\"]";
const LAST_CARD: &str = "[*|aria-label=\"Restore the shipped keys\"]";
const RESET_ALL: &str = "button[*|aria-label=\"Reset every shortcut\"]";

/// Where `selector` is drawn, or a failure that shows the document.
fn rect(harness: &Harness, selector: &str) -> ds::prelude::Rect {
    harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// The top and bottom of what the scroller shows, read before it scrolls: Blitz subtracts an
/// element's own scroll offset from its client rect, so a scrolled scroller reads as moved up by
/// as far as it has scrolled, although it is drawn where it was.
fn view_of(harness: &Harness) -> (f32, f32) {
    let scroller = rect(harness, SCROLLER);
    (
        scroller.origin.y.0,
        scroller.origin.y.0 + scroller.size.height.0,
    )
}

/// Whether all of `selector` lies within `view`.
fn in_view(harness: &Harness, view: (f32, f32), selector: &str) -> bool {
    let found = rect(harness, selector);
    found.origin.y.0 >= view.0 - 0.5 && found.origin.y.0 + found.size.height.0 <= view.1 + 0.5
}

/// Wheel the settings down, as a person would, until `selector` is in `view`, or the scroller
/// stops moving. Whether it got there is the caller's to assert.
fn wheel_to(harness: &mut Harness, view: (f32, f32), selector: &str) {
    let scroller = rect(harness, SCROLLER);
    let at = Point {
        x: ds::prelude::Px(scroller.origin.x.0 + scroller.size.width.0 / 2.0),
        y: ds::prelude::Px(view.0 + 4.0),
    };
    let mut last = rect(harness, selector).origin.y.0;
    for _ in 0..50 {
        if in_view(harness, view, selector) {
            return;
        }
        harness.wheel(at, ds::prelude::Px(0.0), ds::prelude::Px(-120.0));
        harness.advance(ms(20));
        let now = rect(harness, selector).origin.y.0;
        if (now - last).abs() < 0.5 {
            return;
        }
        last = now;
    }
}

/// The Keyboard page, from the Settings window's sidebar.
fn open_page(settings: &mut Harness) {
    click(settings, SIDEBAR_KEYBOARD);
    settle_until(settings, |harness| harness.count(PAGE) == 1);
}

#[test]
fn archive_rebound_in_settings_archives_on_its_new_key_in_the_next_main_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let revisions = (Revisions::new(), Configured::default());

    // The Settings window, on a thread of its own as quire gives each window its own: a window's
    // host (its focus and keyboard) is per thread.
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut settings = settings_window(&store, &dirs, &revisions);
            open_page(&mut settings);

            // A key another action holds is refused, by that action's name, and nothing changes.
            click(&mut settings, CHANGE_ARCHIVE);
            settings.key(Key::Char('s'));
            settings.advance(ms(200));
            let said = settings.text_of(ARCHIVE_SAID).unwrap_or_default();
            assert!(said.contains("Star or unstar"), "{said:?}");
            assert!(
                !dirs.config.join(mail_app::ui::keymap::FILE_NAME).exists(),
                "a refused key wrote the keymap"
            );

            // A free key is taken, and kept.
            click(&mut settings, CHANGE_ARCHIVE);
            settings.key(Key::Char('x'));
            settings.advance(ms(200));
            assert_eq!(settings.count(ARCHIVE_SAID), 0, "{}", settings.html());
            let kept = mail_app::ui::keymap::load(&dirs.config);
            assert_eq!(kept.keys(Shortcut::Archive), ["x"]);
            assert_eq!(kept.action("e", false), None);
            // Escape with nothing waiting leaves the page as it is: it is a page, not a sheet.
            settings.key(Key::Escape);
            settings.advance(ms(300));
            assert_eq!(settings.count(PAGE), 1);
        });
    });

    // The keys are read as mailo opens (quire cannot change a running keymap's overrides yet), so
    // the main window opens after the change. Open the newest conversation, then press the old
    // key: nothing moves.
    let mut main = main_window(&store, &dirs, &revisions);
    main.advance(ms(300));
    let first = ".list .ds-list-item[*|aria-posinset=\"1\"] .ds-thread-sub";
    let rect = main
        .rect(first)
        .unwrap_or_else(|| panic!("{first} is not drawn:\n{}", main.html()));
    main.click(Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    main.advance(ms(300));
    let before = in_inbox(&store);
    assert_eq!(before, INBOX.len());
    main.key(Key::Char('e'));
    main.advance(ms(600));
    assert_eq!(in_inbox(&store), before, "the old key still archived");
    assert_eq!(main.count(".list .ds-thread"), INBOX.len());

    // The new key archives it.
    main.key(Key::Char('x'));
    settle_until(&mut main, |harness| {
        harness.count(".list .ds-thread") == INBOX.len() - 1
    });
    assert_eq!(in_inbox(&store), before - 1, "the new key did not archive");
}

#[test]
fn a_keymap_kept_earlier_is_the_one_settings_shows_and_reset_puts_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let moved = mail_app::ui::keymap::bind(&Default::default(), Shortcut::Archive, "x").unwrap();
    mail_app::ui::keymap::save(&dirs.config, &moved).unwrap();
    let mut settings = settings_window(&store, &dirs, &(Revisions::new(), Configured::default()));

    open_page(&mut settings);
    let row = "[*|data-action=Archive]";
    assert_eq!(
        settings
            .attr(&format!("{row} .kb-key"), "aria-label")
            .as_deref(),
        Some("X"),
        "the page shows the kept key"
    );
    click(&mut settings, "[*|aria-label=\"Reset Archive\"]");
    assert_eq!(
        settings
            .attr(&format!("{row} .kb-key"), "aria-label")
            .as_deref(),
        Some("E")
    );
    assert_eq!(
        mail_app::ui::keymap::load(&dirs.config),
        mail_app::ui::keymap::Keymap::default(),
        "the reset was kept"
    );
    assert_eq!(settings.count("[*|aria-label=\"Reset Archive\"]"), 0);
}

#[test]
fn the_settings_window_scrolls_to_its_last_card() {
    let (_main, mut settings, _dir, _store, _dirs) = open();
    open_page(&mut settings);
    let view = view_of(&settings);
    assert!(
        !in_view(&settings, view, LAST_CARD),
        "the last card was in view unscrolled, so this proves nothing: {:?}",
        rect(&settings, LAST_CARD)
    );
    wheel_to(&mut settings, view, LAST_CARD);
    assert!(
        in_view(&settings, view, LAST_CARD),
        "the wheel stopped with the last card {:?} outside the view {view:?}",
        rect(&settings, LAST_CARD),
    );
    // Reset All is there, and unavailable with nothing changed, so it takes no pointer: what
    // must not be under something else is the row's own name.
    assert_eq!(settings.count(RESET_ALL), 1);
    let title = format!("{LAST_CARD} .ds-field-row-title");
    let name = centre(&settings, &title);
    assert!(
        settings.hits(name, LAST_CARD),
        "the last card is under something else at {name:?}"
    );
}
