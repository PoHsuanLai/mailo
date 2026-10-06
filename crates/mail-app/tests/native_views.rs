//! Saved views, driven the way their user drives them: a search kept with "Save as view", the
//! view in the sidebar, its list grouped and its rows' strip as it was saved, several of its rows
//! picked and archived at once, and the view deleted — in the real window on Blitz
//! (`ds_harness::Harness`).
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

/// A store in `dir` with one account whose archive drops the inbox, and [`INBOX`], unread.
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
             Date: {date}\r\nMessage-ID: <view{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("view{n}"),
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

fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
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

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn press(harness: &mut Harness, selector: &str) {
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(400));
}

fn labelled(label: &str) -> String {
    format!("[*|aria-label=\"{label}\"]")
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    harness.advance(ms(100));
}

/// The message list as drawn, top to bottom: a group header as `# title`, a row as its subject.
fn lines(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".list .ds-list-item[*|aria-posinset]"))
        .filter_map(|n| {
            let at = format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]");
            if harness.count(&format!("{at} .ds-section-header")) > 0 {
                harness
                    .text_of(&format!("{at} .ds-section-header-title"))
                    .map(|t| format!("# {}", t.trim()))
            } else {
                harness.text_of(&format!("{at} .ds-thread-sub"))
            }
        })
        .collect()
}

/// The list child (1-based) that draws `subject`'s row.
fn row_of(harness: &Harness, subject: &str) -> usize {
    (1..=harness.count(".list .ds-list-item[*|aria-posinset]"))
        .find(|n| {
            harness
                .text_of(&format!(
                    ".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread-sub"
                ))
                .as_deref()
                == Some(subject)
        })
        .unwrap_or_else(|| panic!("no row for {subject}:\n{:?}", lines(harness)))
}

fn click_subject(harness: &mut Harness, subject: &str, held: &[Key]) {
    let n = row_of(harness, subject);
    let rect = harness
        .rect(&format!(
            ".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread-sub"
        ))
        .expect("the subject line");
    let at = Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    };
    harness.click_with(at, held);
    harness.advance(ms(300));
}

/// What a row's menu offers, in order, by name: opened with a right click, then let go with
/// Escape.
fn menu_of(harness: &mut Harness, subject: &str) -> Vec<String> {
    let n = row_of(harness, subject);
    row_menu::open_row_menu(
        harness,
        &format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]"),
    );
    let names = row_menu::menu_names(harness);
    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(".ds-menu") == 0);
    names
}

fn place(name: &str) -> String {
    format!("[*|data-place=\"{name}\"]")
}

/// Search for `in:inbox`, save it as "Mine" grouped by read state with Archive and Snooze on
/// hover, and return with the view shown.
fn make_the_view(harness: &mut Harness) {
    press(harness, ".search input");
    type_text(harness, "in:inbox");
    harness.advance(ms(800));
    press(harness, &labelled("Save as view"));
    let sheet = labelled("Saved view");
    assert_eq!(harness.count(&sheet), 1, "Save as view opened no sheet");
    // The name starts as the search; replace it.
    press(harness, &format!("{sheet} .rules-part:nth-child(1) input"));
    // From the start with Delete: on macOS Blitz leaves Backspace in a field to the system's key
    // bindings, which a headless window never gets.
    harness.key(Key::Home);
    for _ in 0.."in:inbox".len() {
        harness.key(Key::Delete);
    }
    type_text(harness, "Mine");
    press(harness, &labelled("Group by: No grouping"));
    assert_eq!(
        harness.count(".ds-menu"),
        1,
        "the grouping menu did not open"
    );
    // Chosen with a press on its row: the menu takes the focus a frame after it opens, and a key
    // sent before it has it goes to the field instead, which the hosted macOS runner shows.
    press(harness, ".ds-menu .ds-menu-item:nth-child(3)");
    harness.advance(ms(300));
    assert_eq!(
        harness.count(&labelled("Group by: Unread, then read")),
        1,
        "the second grouping was not chosen:\n{}",
        harness.html()
    );
    press(harness, &labelled("Offer Snooze on hover"));
    press(harness, &labelled("Offer Archive on hover"));
    press(harness, &labelled("Save new view"));
    harness.advance(ms(600));
    assert_eq!(harness.count(&sheet), 0, "Save left the sheet open");
}

#[test]
fn a_search_saved_as_a_view_is_a_place_grouped_and_offered_as_it_was_saved() {
    let (mut harness, _dir, store) = open();
    // Two read, two not: pick the first two and mark them read.
    click_subject(&mut harness, INBOX[0].1, &[]);
    click_subject(&mut harness, INBOX[1].1, &[Key::Ctrl]);
    harness.key(Key::Char('u'));
    harness.advance(ms(600));
    // Let the selection go. With two picked, the selection bar pushes the list bar's tools past
    // the list's edge at this width, and Save as view with them (FINDINGS, saved views).
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(harness.count(&place("Mine")), 0);
    make_the_view(&mut harness);

    // Kept in the store, as the view the sheet described.
    let views = store.views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Mine");
    assert_eq!(views[0].group_by, Some(GroupKey::Read));
    assert_eq!(views[0].hover, vec![OpKind::Snooze, OpKind::Archive]);

    // In the sidebar, and shown, the search box empty again.
    assert_eq!(
        harness.count(&place("Mine")),
        1,
        "the view is not in the sidebar"
    );
    assert_eq!(harness.attr(".search input", "value").as_deref(), Some(""));
    let title = harness.text_of(".list-title .ds-label").unwrap_or_default();
    assert!(title.starts_with("Mine"), "{title}");

    // Grouped by read state, the list order kept inside each band.
    assert_eq!(
        lines(&harness),
        vec![
            "# Unread".to_owned(),
            INBOX[2].1.to_owned(),
            INBOX[3].1.to_owned(),
            "# Read".to_owned(),
            INBOX[0].1.to_owned(),
            INBOX[1].1.to_owned(),
        ]
    );

    // The row's actions are the view's (Archive and Snooze), not the usual ones, beside what
    // every row's menu has.
    let mine = menu_of(&mut harness, INBOX[2].1);
    assert!(
        mine.contains(&"Snooze…".to_owned()) && mine.contains(&"Archive".to_owned()),
        "{mine:?}"
    );
    for usual in ["Trash", "Pin", "Label…", "Forward"] {
        assert!(
            !mine.contains(&usual.to_owned()),
            "{usual} in the view: {mine:?}"
        );
    }

    // Somewhere else, the usual strip and no bands.
    press(&mut harness, &place("Inbox"));
    assert!(
        !lines(&harness).iter().any(|line| line.starts_with('#')),
        "{:?}",
        lines(&harness)
    );
    let usual = menu_of(&mut harness, INBOX[2].1);
    for kind in ["Trash", "Pin", "Label…", "Forward"] {
        assert!(
            usual.contains(&kind.to_owned()),
            "no {kind} in the inbox: {usual:?}"
        );
    }
}

#[test]
fn several_rows_picked_in_a_view_are_archived_as_one_gesture() {
    let (mut harness, _dir, store) = open();
    make_the_view(&mut harness);
    press(&mut harness, &place("Mine"));
    let before = lines(&harness).len();
    click_subject(&mut harness, INBOX[1].1, &[]);
    click_subject(&mut harness, INBOX[3].1, &[Key::Ctrl]);
    let picked: Vec<usize> = (1..=harness.count(".list .ds-list-item[*|aria-posinset]"))
        .filter(|n| {
            harness
                .attr(
                    &format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-row"),
                    "aria-selected",
                )
                .as_deref()
                == Some("true")
        })
        .collect();
    assert_eq!(picked.len(), 2, "two rows are not drawn picked");
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    let left = lines(&harness);
    assert!(!left.contains(&INBOX[1].1.to_owned()), "{left:?}");
    assert!(!left.contains(&INBOX[3].1.to_owned()), "{left:?}");
    let rows_before = before - left.len();
    assert!(rows_before >= 2, "two rows left the view: {left:?}");
    let archived = store
        .threads(
            &Query {
                filter: Filter::InMailbox(MailboxRole::Archive),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .items
        .len();
    assert_eq!(archived, 2);
    // One Ctrl Z puts both back in the view.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(1500));
    let back = lines(&harness);
    assert!(back.contains(&INBOX[1].1.to_owned()), "{back:?}");
    assert!(back.contains(&INBOX[3].1.to_owned()), "{back:?}");
}

#[test]
fn a_view_deleted_from_its_editor_leaves_the_sidebar_and_the_store() {
    let (mut harness, _dir, store) = open();
    make_the_view(&mut harness);
    assert_eq!(store.views().unwrap().len(), 1);
    press(&mut harness, &labelled("Edit view"));
    let sheet = labelled("Saved view");
    assert_eq!(harness.count(&sheet), 1, "Edit view opened no sheet");
    assert_eq!(
        harness.count(&labelled("Group by: Unread, then read")),
        1,
        "the editor did not open on the view's grouping"
    );
    press(&mut harness, &labelled("Delete view"));
    harness.advance(ms(600));
    assert_eq!(harness.count(&sheet), 0, "Delete left the sheet open");
    assert_eq!(store.views().unwrap(), Vec::<View>::new());
    assert_eq!(
        harness.count(&place("Mine")),
        0,
        "the view is still in the sidebar"
    );
    let title = harness.text_of(".list-title .ds-label").unwrap_or_default();
    assert!(title.starts_with("Inbox"), "{title}");
}
