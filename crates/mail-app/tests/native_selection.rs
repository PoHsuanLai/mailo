//! Several conversations picked at once, driven the way their user drives them: Shift-click,
//! Ctrl-click, Shift+j/k and Ctrl A in the real window on Blitz (`ds_native::Harness`), and an
//! action on the selection taken back by one Ctrl Z.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config.

use dioxus::prelude::Modifiers;
use ds::{Key, Point};
use ds_native::{Harness, HarnessConfig, NetPolicy, PrintOutcome, Viewport};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The seeded inbox, newest first, as the list draws it: (sender, subject).
const INBOX: [(&str, &str); 5] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
    ("edsger@example.test", "Notes from the review"),
    ("barbara@example.test", "The shared drive is full"),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A store in `dir` with one IMAP-like account (archive drops the inbox, as on a label server)
/// and [`INBOX`].
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
             Date: {date}\r\nMessage-ID: <pick{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("pick{n}"),
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
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    (harness, dir, store)
}

/// The `n`th row of the list (1-based).
fn row(n: usize) -> String {
    format!(".ds-list > .row:nth-child({n}) .ds-row")
}

/// Where a person clicks the `n`th row: the start of its subject line, clear of the hover strip.
fn subject_of(harness: &Harness, n: usize) -> Point {
    let subject = format!("{} .ds-row-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    Point {
        x: ds::Px(rect.origin.x.0 + 24.0),
        y: ds::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    }
}

fn click_row(harness: &mut Harness, n: usize, held: Modifiers) {
    let at = subject_of(harness, n);
    harness.click_with(at, held);
    harness.advance(ms(300));
}

/// Which rows (1-based) are drawn selected, top to bottom.
fn selected(harness: &Harness) -> Vec<usize> {
    (1..=harness.count(".ds-list > .row"))
        .filter(|n| harness.attr(&row(*n), "aria-selected").as_deref() == Some("true"))
        .collect()
}

/// The subjects of the list's rows, top to bottom.
fn subjects(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".ds-list > .row"))
        .filter_map(|n| harness.text_of(&format!("{} .ds-row-sub", row(n))))
        .collect()
}

/// What the list bar says about the selection, if anything.
fn said(harness: &Harness) -> Option<String> {
    let html = harness.html();
    let before = &html[..html.find(" selected</span>")?];
    let count: String = before
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(format!("{count} selected"))
}

fn inbox_count(store: &SqliteStore) -> usize {
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

#[test]
fn shift_click_picks_the_range_from_the_open_row() {
    let (mut harness, _dir, _store) = open();
    click_row(&mut harness, 2, Modifiers::empty());
    assert_eq!(selected(&harness), vec![2], "a plain click selects its row");
    assert_eq!(said(&harness), None, "one open row is not a selection");
    click_row(&mut harness, 4, Modifiers::SHIFT);
    assert_eq!(selected(&harness), vec![2, 3, 4]);
    assert_eq!(said(&harness).as_deref(), Some("3 selected"));
    // Measured again from the same anchor, upward this time.
    click_row(&mut harness, 1, Modifiers::SHIFT);
    assert_eq!(selected(&harness), vec![1, 2]);
    // A plain click lets the selection go.
    click_row(&mut harness, 5, Modifiers::empty());
    assert_eq!(selected(&harness), vec![5]);
    assert_eq!(said(&harness), None);
}

#[test]
fn ctrl_click_adds_and_takes_away_one_row_at_a_time() {
    let (mut harness, _dir, _store) = open();
    click_row(&mut harness, 1, Modifiers::empty());
    click_row(&mut harness, 3, Modifiers::CONTROL);
    assert_eq!(
        selected(&harness),
        vec![1, 3],
        "the open row and the added one"
    );
    click_row(&mut harness, 5, Modifiers::CONTROL);
    assert_eq!(selected(&harness), vec![1, 3, 5]);
    click_row(&mut harness, 1, Modifiers::CONTROL);
    assert_eq!(
        selected(&harness),
        vec![3, 5],
        "a second Ctrl-click takes it out"
    );
    assert_eq!(said(&harness).as_deref(), Some("2 selected"));
    // The reader still shows what was opened: Ctrl-click picks, it does not open.
    let reader = harness.text_of(".reader").unwrap_or_default();
    assert!(
        reader.contains("The body of Flight to the conference."),
        "{reader}"
    );
}

#[test]
fn ctrl_a_picks_every_row_and_escape_lets_them_go() {
    let (mut harness, _dir, _store) = open();
    assert_eq!(selected(&harness), Vec::<usize>::new());
    harness.chord(&[Key::Ctrl], Key::Char('a'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![1, 2, 3, 4, 5]);
    assert_eq!(said(&harness).as_deref(), Some("5 selected"));
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(selected(&harness), Vec::<usize>::new());
    assert_eq!(said(&harness), None);
}

#[test]
fn shift_j_and_shift_k_move_the_end_of_the_range() {
    let (mut harness, _dir, _store) = open();
    click_row(&mut harness, 2, Modifiers::empty());
    harness.chord(&[Key::Shift], Key::Char('j'));
    harness.chord(&[Key::Shift], Key::Char('j'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![2, 3, 4]);
    harness.chord(&[Key::Shift], Key::Char('k'));
    harness.advance(ms(300));
    assert_eq!(selected(&harness), vec![2, 3]);
    // Nothing was opened on the way: the reader is on the row that was clicked.
    let reader = harness.text_of(".reader").unwrap_or_default();
    assert!(
        reader.contains("The body of The invoice for September."),
        "{reader}"
    );
}

#[test]
fn archiving_a_selection_is_one_gesture_and_one_undo_puts_it_all_back() {
    let (mut harness, _dir, store) = open();
    let before = inbox_count(&store);
    assert_eq!(before, INBOX.len());
    click_row(&mut harness, 2, Modifiers::empty());
    click_row(&mut harness, 4, Modifiers::SHIFT);
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    assert_eq!(
        inbox_count(&store),
        before - 3,
        "the archive did not reach all three"
    );
    assert_eq!(
        subjects(&harness),
        vec![
            "Flight to the conference".to_owned(),
            "The shared drive is full".to_owned(),
        ]
    );
    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Archived · 3 conversations"), "{toast}");
    // One Ctrl Z: every one of the three is back, not only the last.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(1500));
    assert_eq!(inbox_count(&store), before, "the undo left some archived");
    let want: Vec<String> = INBOX.iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(subjects(&harness), want);
    // And that was the whole entry: a second Ctrl Z has nothing of this left to take back.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(inbox_count(&store), before);
}

#[test]
fn a_button_on_the_selection_bar_acts_on_every_picked_row() {
    let (mut harness, _dir, store) = open();
    click_row(&mut harness, 1, Modifiers::empty());
    click_row(&mut harness, 3, Modifiers::CONTROL);
    let star = "[*|aria-label=\"Star the 2 selected\"]";
    let at = harness
        .centre(star)
        .unwrap_or_else(|| panic!("no Star on the selection bar:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(600));
    let starred: Vec<String> = subjects_with(&store, Star::Starred);
    assert_eq!(
        starred,
        vec![
            "Flight to the conference".to_owned(),
            "Lunch on Thursday".to_owned(),
        ]
    );
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(subjects_with(&store, Star::Starred), Vec::<String>::new());
}

/// The subjects of the inbox's conversations with `star`, newest first.
fn subjects_with(store: &SqliteStore, star: Star) -> Vec<String> {
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
        .into_iter()
        .filter(|thread| thread.star == star)
        .map(|thread| thread.subject)
        .collect()
}

#[test]
fn the_selection_does_not_follow_to_another_place() {
    let (mut harness, _dir, _store) = open();
    harness.chord(&[Key::Ctrl], Key::Char('a'));
    harness.advance(ms(300));
    assert_eq!(said(&harness).as_deref(), Some("5 selected"));
    let place = |name: &str| format!("[*|data-place=\"{name}\"]");
    let at = harness
        .centre(&place("Starred"))
        .unwrap_or_else(|| panic!("no Starred place:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(600));
    let at = harness.centre(&place("Inbox")).expect("the Inbox place");
    harness.click(at);
    harness.advance(ms(600));
    assert_eq!(subjects(&harness).len(), INBOX.len(), "back in the inbox");
    assert_eq!(selected(&harness), Vec::<usize>::new());
    assert_eq!(said(&harness), None);
}
