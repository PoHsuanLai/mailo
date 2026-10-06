//! The window's layout and chrome against the real window on Blitz (`ds_harness::Harness`): the
//! hover strip under a resting pointer, the reader's least width, the sidebar's way back and its
//! footer and account tiles at the sidebar's least width.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config.

use ds::prelude::{Point, Px, Rect};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
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

/// The seeded inbox, newest first, as the list draws it: (sender, subject).
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

fn rect(harness: &Harness, selector: &str) -> Rect {
    harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// The first row of the list, as the element a click lands on.
const ROW: &str = ".list .ds-list-item[*|aria-posinset=\"1\"]";

fn left(r: &Rect) -> f32 {
    r.origin.x.0
}
fn right(r: &Rect) -> f32 {
    r.origin.x.0 + r.size.width.0
}
fn top(r: &Rect) -> f32 {
    r.origin.y.0
}
fn bottom(r: &Rect) -> f32 {
    r.origin.y.0 + r.size.height.0
}

/// Whether `inner` lies within `outer`, to half a pixel.
fn within(inner: &Rect, outer: &Rect) -> bool {
    left(inner) >= left(outer) - 0.5
        && right(inner) <= right(outer) + 0.5
        && top(inner) >= top(outer) - 0.5
        && bottom(inner) <= bottom(outer) + 0.5
}

/// Drag the sidebar's divider `dx` pixels, as a person drags it.
fn drag_sidebar(harness: &mut Harness, dx: f32) {
    let divider = centre(harness, ".ds-split-divider");
    harness.drag(
        divider,
        Point {
            x: Px(divider.x.0 + dx),
            y: divider.y,
        },
        8,
    );
    harness.advance(ms(300));
}

#[test]
fn a_pointer_resting_on_the_strip_opens_the_thread_instead_of_pressing_it() {
    let (mut harness, _dir) = open(1200, spaces(1));
    let rows = harness.count(".list .ds-thread");
    let readers = harness.count(".reader .ds-empty-state");
    assert_eq!(
        (rows, readers),
        (4, 1),
        "the fixture is not what the test assumes"
    );
    // The row's middle-right, where the strip is laid out.
    let at = centre(&harness, &format!("{ROW} .ds-strip"));
    harness.pointer_move(at);
    harness.advance(ms(100));
    harness.click(at);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".list .ds-thread"),
        rows,
        "a click before the strip armed acted on the thread"
    );
    assert_eq!(
        harness.count(".reader .ds-empty-state"),
        readers - 1,
        "the click did not open the thread"
    );
}

#[test]
fn the_strip_arms_once_the_pointer_has_dwelled() {
    let (mut harness, _dir) = open(1200, spaces(1));
    let strip = format!("{ROW} .ds-strip");
    let at = centre(&harness, &strip);
    let shown = format!("{strip}[*|data-shown=visible]");
    assert_eq!(harness.count(&shown), 0, "a strip at rest is not held down");
    harness.pointer_move(at);
    harness.advance(ms(100));
    assert_eq!(harness.count(&shown), 0, "armed before the dwell");
    harness.advance(ms(300));
    assert_eq!(harness.count(&shown), 1, "never armed");
    harness.pointer_move(Point {
        x: Px(2.0),
        y: Px(2.0),
    });
    harness.advance(ms(100));
    assert_eq!(
        harness.count(&shown),
        0,
        "still armed after the pointer left"
    );
}

#[test]
fn a_hidden_sidebar_can_be_shown_again_from_the_list() {
    const HIDE: &str = ".side-foot [aria-label=\"Hide sidebar\"]";
    const SHOW: &str = ".list-head [aria-label=\"Show sidebar\"]";
    let (mut harness, _dir) = open(1200, spaces(1));
    assert_eq!(
        harness.count(SHOW),
        0,
        "the way back is offered while the sidebar is shown"
    );
    let hide = centre(&harness, HIDE);
    harness.click(hide);
    harness.advance(ms(400));
    assert_eq!(
        harness.count(SHOW),
        1,
        "no way back while the sidebar is hidden"
    );
    let show = centre(&harness, SHOW);
    harness.click(show);
    harness.advance(ms(400));
    assert_eq!(
        harness.count(SHOW),
        0,
        "the way back stayed after it was taken"
    );
    assert_eq!(harness.count(HIDE), 1, "the sidebar did not come back");
    assert!(
        rect(&harness, ".side").size.width.0 > 100.0,
        "the sidebar is back but not drawn"
    );
}

#[test]
fn every_footer_control_stays_inside_the_sidebar_at_its_least_width() {
    let (mut harness, _dir) = open(1200, spaces(2));
    drag_sidebar(&mut harness, -60.0);
    let side = rect(&harness, ".side");
    assert!(
        side.size.width.0 < 200.0,
        "the sidebar did not reach its least: {}px",
        side.size.width.0
    );
    let mut seen = 0;
    for child in 1..=8 {
        let selector = format!(".side-foot > :nth-child({child})");
        if let Some(found) = harness.rect(&selector) {
            seen += 1;
            assert!(
                within(&found, &side),
                "{selector} is {found:?}, outside {side:?}"
            );
        }
    }
    assert!(seen >= 4, "only {seen} footer controls are drawn");
    let toggle = rect(&harness, ".side-foot > :last-child");
    assert!(
        within(&toggle, &side),
        "the sidebar toggle is clipped: {toggle:?} in {side:?}"
    );
}

#[test]
fn the_list_s_title_gives_way_to_its_tools_in_a_narrow_list() {
    // The narrowest window the list keeps its least width in: its header is at its tightest.
    let (harness, _dir) = open(760, spaces(1));
    let column = rect(&harness, ".list-col");
    let title = rect(&harness, ".list-title");
    let tools = rect(&harness, ".bar-tools");
    assert!(
        right(&title) <= left(&tools) + 0.5,
        "the title {title:?} runs under the tools {tools:?}"
    );
    assert!(
        within(&tools, &column),
        "the tools {tools:?} are clipped by the list {column:?}"
    );
}
