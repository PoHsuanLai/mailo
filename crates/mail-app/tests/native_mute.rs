//! Muting a conversation, driven the way its user drives it in the real window on Blitz
//! (`ds_harness::Harness`): the row's Mute button, the reader's, `m` on the open conversation and
//! on a selection, and Ctrl Z taking each back.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
#[path = "support/row_menu.rs"]
mod row_menu;
use drive::Drive;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
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
             Date: {date}\r\nMessage-ID: <mute{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("mute{n}"),
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

/// The window over a freshly seeded store, first frame drawn, with the store it writes. The
/// `TempDir` must outlive both.
fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    (harness, dir, store)
}

/// The `n`th row of the list (1-based).
fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Click the `n`th row where a person reads it: the start of its subject line, clear of the
/// hover strip.
fn click_row(harness: &mut Harness, n: usize, held: &[Key]) {
    let subject = format!("{} .ds-thread-sub", row(n));
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
        .filter(|n| harness.count(&format!("{} .mute-mark", row(*n))) > 0)
        .collect()
}

fn toast(harness: &Harness) -> String {
    harness.text_of(".ds-toast").unwrap_or_default()
}

/// The row's Mute mutes it and marks it, and Ctrl Z unmutes it. Then the reader mutes the open
/// conversation and says so, and `m` unmutes it, each its own gesture to take back.
#[test]
fn the_row_s_menu_and_the_reader_mute_and_ctrl_z_and_m_unmute() {
    let (mut harness, _dir, store) = open();
    assert_eq!(muted(&store), Vec::<String>::new(), "muted at the start");
    assert_eq!(marked(&harness), Vec::<usize>::new(), "marked at the start");

    // The row's menu.
    click_row(&mut harness, 1, &[]);
    row_menu::row_action(&mut harness, &row(2), "Mute");
    harness.advance(ms(600));

    assert_eq!(muted(&store), vec![INBOX[1].1.to_owned()], "the row's Mute");
    assert_eq!(marked(&harness), vec![2], "the row shows it is muted");
    assert!(
        toast(&harness).contains("Muted"),
        "the row's Mute: {}",
        toast(&harness)
    );
    // Muting leaves the conversation where it is: it is about the replies still to come.
    assert_eq!(
        harness.count(".list .ds-thread"),
        INBOX.len(),
        "muting moved the conversation"
    );

    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "Ctrl Z unmuted it");
    assert_eq!(
        marked(&harness),
        Vec::<usize>::new(),
        "the mark outlived Ctrl Z"
    );

    // The reader.
    click_row(&mut harness, 3, &[]);
    assert_eq!(
        harness.count(".reader-head .muted-note"),
        0,
        "the reader says muted before it is"
    );

    let tool = "[*|aria-label=\"Mute this conversation\"]";
    harness.click(centre(&harness, tool));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[2].1.to_owned()],
        "the reader's Mute"
    );
    let note = harness
        .text_of(".reader-head .muted-note")
        .unwrap_or_default();
    assert!(
        note.contains("Muted"),
        "the reader shows it is muted: {note:?}"
    );
    assert_eq!(
        harness
            .attr(
                "[*|aria-label=\"Unmute this conversation\"]",
                "aria-pressed"
            )
            .as_deref(),
        Some("true"),
        "the tool is pressed while it is muted"
    );

    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "`m` unmuted it");
    assert_eq!(
        harness.count(".reader-head .muted-note"),
        0,
        "the reader says muted after `m`"
    );

    // Each was its own gesture: Ctrl Z takes back the unmute, then the mute.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[2].1.to_owned()],
        "Ctrl Z took back the unmute"
    );
    harness.chord(&[Key::Ctrl], Key::Char('z'));
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

    harness.chord(&[Key::Ctrl], Key::Char('z'));
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
    click_row(&mut harness, 2, &[Key::Ctrl]);
    let button = "[*|aria-label=\"Mute the 2 selected\"]";
    harness.click(centre(&harness, button));
    harness.advance(ms(600));
    let two: Vec<String> = INBOX[..2].iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(muted(&store), two, "mutes the rest, never a toggle each");

    // Its undo takes back only what it did: the second stays muted.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(
        muted(&store),
        vec![INBOX[1].1.to_owned()],
        "the bar's undo took back more than it did"
    );
}
