//! Custom keyboard shortcuts, driven the way their user drives them in the real window on Blitz
//! (`ds_harness::Harness`): the settings' Keyboard section, Change on Archive, a key that another
//! action holds refused by that action's name, a free key taken, and then the new key archiving
//! and the old one doing nothing.
//!
//! The window is handed a pair of `TempDir` directories, so the keymap it keeps lands there and
//! nowhere else; the store is seeded in a `TempDir` too.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use mail_app::ui::appearance::WindowDirs;
use mail_app::ui::view::Shortcut;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b7"));

/// The window a laptop gives it: the settings' last card, Keyboard, is below the fold and is
/// reached by scrolling the settings.
const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
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
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&caps).unwrap()],
        )
        .unwrap();
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
            ACCOUNT,
            MailboxRef {
                account: ACCOUNT,
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

/// The window over a freshly seeded store and a pair of empty directories, first frame drawn.
/// The `TempDir` must outlive the rest.
fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>, WindowDirs) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        Some(dirs.clone()),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    settle_until(&mut harness, |harness| {
        harness.count(".list .ds-thread") == INBOX.len()
    });
    (harness, dir, store, dirs)
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

const SHEET: &str = "[*|aria-label=\"Keyboard shortcuts\"][*|role=dialog]";
const CHANGE_ARCHIVE: &str = "[*|aria-label=\"Change the key for Archive\"]";

const EDITOR: &str = "[*|aria-label=\"Edit this Space\"]";
const SCROLLER: &str = ".ed-scroll";
const LAST_CARD: &str = "[*|aria-label=\"Keyboard shortcuts\"]";
const KEYBOARD: &str = "button[*|aria-label=\"Keyboard shortcuts\"]";

/// Where `selector` is drawn, or a failure that shows the document.
fn rect(harness: &Harness, selector: &str) -> ds::prelude::Rect {
    harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// The top and bottom of what the settings' scroller shows: from the sheet's top to its foot,
/// neither of which scrolls. Not the scroller's own rect: Blitz subtracts an element's own
/// scroll offset from its client rect, so a scrolled scroller reads as moved up by as far as it
/// has scrolled, although it is drawn where it was.
fn settings_view(harness: &Harness) -> (f32, f32) {
    (
        rect(harness, EDITOR).origin.y.0,
        rect(harness, ".ed-foot").origin.y.0,
    )
}

/// Whether all of `selector` lies within what the settings' scroller shows.
fn in_view(harness: &Harness, selector: &str) -> bool {
    let (top, bottom) = settings_view(harness);
    let found = rect(harness, selector);
    found.origin.y.0 >= top - 0.5 && found.origin.y.0 + found.size.height.0 <= bottom + 0.5
}

/// Wheel the settings down, as a person would, until `selector` is in view, or the scroller
/// stops moving. Whether it got there is the caller's to assert.
fn wheel_to(harness: &mut Harness, selector: &str) {
    let at = centre(harness, SCROLLER);
    let mut last = rect(harness, selector).origin.y.0;
    for _ in 0..50 {
        if in_view(harness, selector) {
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

/// Open the settings, then their Keyboard section's sheet.
fn open_sheet(harness: &mut Harness) {
    click(harness, "[*|aria-label=\"Space settings\"]");
    settle_until(harness, |harness| harness.count(KEYBOARD) == 1);
    wheel_to(harness, KEYBOARD);
    click(harness, KEYBOARD);
    settle_until(harness, |harness| harness.count(SHEET) == 1);
    assert_eq!(
        harness.count(SHEET),
        1,
        "the sheet opened:\n{}",
        harness.html()
    );
}

#[test]
fn archive_rebound_in_the_settings_archives_on_its_new_key_and_not_its_old_one() {
    let (mut harness, _dir, store, dirs) = open();
    open_sheet(&mut harness);

    // A key another action holds is refused, by that action's name, and nothing changes.
    click(&mut harness, CHANGE_ARCHIVE);
    harness.key(Key::Char('s'));
    harness.advance(ms(200));
    let said = harness.text_of(".rules [*|role=alert]").unwrap_or_default();
    assert!(said.contains("Star or unstar"), "{said:?}");
    assert!(
        !dirs.config.join(mail_app::ui::keymap::FILE_NAME).exists(),
        "a refused key wrote the keymap"
    );

    // A free key is taken, and kept.
    click(&mut harness, CHANGE_ARCHIVE);
    harness.key(Key::Char('x'));
    harness.advance(ms(200));
    assert_eq!(
        harness.count(".rules [*|role=alert]"),
        0,
        "{}",
        harness.html()
    );
    let kept = mail_app::ui::keymap::load(&dirs.config);
    assert_eq!(kept.keys(Shortcut::Archive), ["x"]);
    assert_eq!(kept.action("e", false), None);

    // Esc closes the sheet, and Esc again the settings under it.
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(harness.count(SHEET), 0);
    harness.key(Key::Escape);
    harness.advance(ms(300));

    // Open the newest conversation, then press the old key: nothing moves.
    let first = ".list .ds-list > .ds-list-item:nth-child(1) .ds-thread-sub";
    let rect = harness
        .rect(first)
        .unwrap_or_else(|| panic!("{first} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
    let before = in_inbox(&store);
    assert_eq!(before, INBOX.len());
    harness.key(Key::Char('e'));
    harness.advance(ms(600));
    assert_eq!(in_inbox(&store), before, "the old key still archived");
    assert_eq!(harness.count(".list .ds-thread"), INBOX.len());

    // The new key archives it.
    harness.key(Key::Char('x'));
    settle_until(&mut harness, |harness| {
        harness.count(".list .ds-thread") == INBOX.len() - 1
    });
    assert_eq!(in_inbox(&store), before - 1, "the new key did not archive");
    assert_eq!(harness.count(".list .ds-thread"), INBOX.len() - 1);
}

#[test]
fn a_keymap_kept_earlier_is_the_one_a_new_window_answers_to_and_reset_puts_it_back() {
    let (mut harness, _dir, store, dirs) = open();
    let moved = mail_app::ui::keymap::bind(&Default::default(), Shortcut::Archive, "x").unwrap();
    mail_app::ui::keymap::save(&dirs.config, &moved).unwrap();
    // A second window over the same directories reads it.
    drop(harness);
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        Some(dirs.clone()),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    harness = Harness::new(
        mail_app::ui::native::root,
        HarnessConfig::new(VIEW)
            .with_net(NetPolicy::Local)
            .with_clock(Clock::Virtual)
            .with_contexts(contexts),
    );
    settle_until(&mut harness, |harness| {
        harness.count(".list .ds-thread") == INBOX.len()
    });

    open_sheet(&mut harness);
    let row = "[*|data-action=Archive]";
    assert_eq!(
        harness.attr(&format!("{row} .kb-key"), "title").as_deref(),
        Some("X"),
        "the sheet shows the kept key"
    );
    click(&mut harness, "[*|aria-label=\"Reset Archive\"]");
    assert_eq!(
        harness.attr(&format!("{row} .kb-key"), "title").as_deref(),
        Some("E")
    );
    assert_eq!(
        mail_app::ui::keymap::load(&dirs.config),
        mail_app::ui::keymap::Keymap::default(),
        "the reset was kept"
    );
    assert_eq!(harness.count("[*|aria-label=\"Reset Archive\"]"), 0);
}

#[test]
fn the_settings_scroll_to_their_last_card_in_an_800_px_window() {
    let (mut harness, _dir, _store, _dirs) = open();
    click(&mut harness, "[*|aria-label=\"Space settings\"]");
    settle_until(&mut harness, |harness| harness.count(LAST_CARD) == 1);
    // Unscrolled, the scroller's own rect is the view `settings_view` reads from the sheet and
    // its foot, and the last card is below it.
    let scroller = rect(&harness, SCROLLER);
    let (top, bottom) = settings_view(&harness);
    assert!(
        (scroller.origin.y.0 - top).abs() < 0.5
            && (scroller.origin.y.0 + scroller.size.height.0 - bottom).abs() < 0.5,
        "the scroller {scroller:?} is not the sheet above its foot: {top}..{bottom}"
    );
    assert!(
        !in_view(&harness, LAST_CARD),
        "the last card was in view unscrolled, so this proves nothing: {:?}",
        rect(&harness, LAST_CARD)
    );

    wheel_to(&mut harness, LAST_CARD);
    assert!(
        in_view(&harness, LAST_CARD),
        "the wheel stopped with the last card {:?} outside the view {:?}",
        rect(&harness, LAST_CARD),
        settings_view(&harness)
    );
    let button = centre(&harness, KEYBOARD);
    assert!(
        harness.hits(button, KEYBOARD),
        "the Keyboard button is under something else at {button:?}"
    );
}
