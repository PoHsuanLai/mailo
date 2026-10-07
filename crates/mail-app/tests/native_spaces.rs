//! Deleting a Space from its menu, against the real window on Blitz: the Space's name opens the
//! menu at the pointer, Delete Space… asks in a popover where the menu stood, and only the
//! popover's button removes the Space. With one Space the menu has no Delete row.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere.

use ds::base::press::PointerButton;
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::{Drive, Key};
use mail_app::ui::space::{Space, Spaces};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

/// A viewport `width` by 700 at 100% scale.
fn view(width: u32) -> Viewport {
    Viewport {
        width,
        height: 700,
        scale_percent: 100,
    }
}

/// The seeded inbox, newest first: (sender, subject).
const INBOX: [(&str, &str); 4] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
    ("edsger@example.test", "Notes from the review"),
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
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&caps).unwrap()
            ],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <lay{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("lay{n}"),
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

/// `count` Spaces, the first on screen, each showing every account.
fn spaces(count: usize) -> Spaces {
    Spaces {
        spaces: (1..=count)
            .map(|n| Space {
                name: format!("Space {n}"),
                ..Space::default()
            })
            .collect(),
        current: 0,
        recall: Default::default(),
    }
}

/// The window, `width` wide, over a freshly seeded store and `spaces`, first frame drawn. The
/// `TempDir` must outlive it.
fn open(width: u32, spaces: Spaces) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        spaces,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(view(width))
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    (harness, dir)
}

const DOTS: &str = ".space-dots > *";
const NAME: &str = ".side-head";
const MENU: &str = ".ds-menu-item";
/// The menu's last row: Delete Space… while another Space remains, New Space otherwise.
const LAST_ROW: &str = ".ds-menu-item:last-child";
const ASKING: &str = ".space-delete";
const CONFIRM: &str = ".space-delete [*|aria-label=\"Delete Space\"]";

/// A right click on `selector`: the Space's name opens its menu only so.
fn right_press(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.press(at, PointerButton::Secondary);
    harness.advance(ms(400));
}

fn press(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(400));
}

#[test]
fn deleting_a_space_asks_first_and_then_removes_it() {
    let (mut harness, _dir) = open(1200, spaces(2));
    assert_eq!(harness.count(DOTS), 2, "the fixture is not two Spaces");
    right_press(&mut harness, NAME);
    assert!(
        harness.count(MENU) > 0,
        "the Space's name did not open its menu"
    );
    assert_eq!(
        harness.text_of(LAST_ROW).as_deref().map(str::trim),
        Some("Delete Space\u{2026}")
    );
    press(&mut harness, LAST_ROW);
    assert_eq!(harness.count(ASKING), 1, "Delete Space did not ask");
    assert_eq!(harness.count(MENU), 0, "the menu stayed under the question");
    assert_eq!(
        harness.count(DOTS),
        2,
        "the Space went before it was confirmed"
    );
    press(&mut harness, CONFIRM);
    assert_eq!(
        harness.count(DOTS),
        1,
        "confirming did not remove the Space"
    );
    assert_eq!(
        harness.count(ASKING),
        0,
        "the question stayed after confirming"
    );
}

#[test]
fn a_plain_click_on_the_spaces_name_opens_nothing() {
    let (mut harness, _dir) = open(1200, spaces(2));
    press(&mut harness, NAME);
    assert_eq!(harness.count(MENU), 0, "a left click opened the Space's menu");
    right_press(&mut harness, NAME);
    assert!(harness.count(MENU) > 0, "a right click did not open it");
}

#[test]
fn escape_keeps_the_space() {
    let (mut harness, _dir) = open(1200, spaces(2));
    right_press(&mut harness, NAME);
    press(&mut harness, LAST_ROW);
    assert_eq!(harness.count(ASKING), 1, "Delete Space did not ask");
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(harness.count(ASKING), 0, "Esc did not close the question");
    assert_eq!(
        harness.count(DOTS),
        2,
        "closing the question removed a Space"
    );
}

#[test]
fn the_only_space_has_no_delete_row() {
    let (mut harness, _dir) = open(1200, spaces(1));
    right_press(&mut harness, NAME);
    assert!(
        harness.count(MENU) > 0,
        "the Space's name did not open its menu"
    );
    assert_eq!(
        harness.text_of(LAST_ROW).as_deref().map(str::trim),
        Some("New Space"),
        "the last Space's menu offers Delete"
    );
    assert_eq!(harness.count(DOTS), 1, "the last Space was removed");
}
