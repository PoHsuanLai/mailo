//! Muting a conversation, driven the way its user drives it in the real window on Blitz
//! (`ds_native::Harness`): the row's Mute button, the reader's, `m` on the open conversation and
//! on a selection, and Ctrl Z taking each back.
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
             Date: {date}\r\nMessage-ID: <mute{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
    format!(".ds-list > .row:nth-child({n})")
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Click the `n`th row where a person reads it: the start of its subject line, clear of the
/// hover strip.
fn click_row(harness: &mut Harness, n: usize, held: Modifiers) {
    let subject = format!("{} .ds-row-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click_with(
        Point {
            x: ds::Px(rect.origin.x.0 + 24.0),
            y: ds::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
        },
        held,
    );
    harness.advance(ms(300));
}

/// Which conversations are muted, by subject, newest first, as the store has them.
fn muted(store: &SqliteStore) -> Vec<String> {
    let query = Query {
        filter: Filter::Account(ACCOUNT),
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
    (1..=harness.count(".ds-list > .row"))
        .filter(|n| harness.count(&format!("{} .mute-mark", row(*n))) > 0)
        .collect()
}

fn toast(harness: &Harness) -> String {
    harness.text_of(".ds-toast").unwrap_or_default()
}

#[test]
fn the_row_s_mute_button_mutes_it_marks_it_and_ctrl_z_unmutes_it() {
    let (mut harness, _dir, store) = open();
    assert_eq!(muted(&store), Vec::<String>::new());
    assert_eq!(marked(&harness), Vec::<usize>::new());

    click_row(&mut harness, 1, Modifiers::empty());
    let second = format!("{} .ds-row-sub", row(2));
    harness.pointer_move(centre(&harness, &second));
    harness.advance(ms(300));
    let mute = format!("{} .ds-strip [*|data-op=mute]", row(2));
    harness.click(centre(&harness, &mute));
    harness.advance(ms(600));

    assert_eq!(muted(&store), vec![INBOX[1].1.to_owned()]);
    assert_eq!(marked(&harness), vec![2], "the row shows it is muted");
    assert!(toast(&harness).contains("Muted"), "{}", toast(&harness));
    // Muting leaves the conversation where it is: it is about the replies still to come.
    assert_eq!(harness.count(".ds-list > .row"), INBOX.len());

    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "Ctrl Z unmuted it");
    assert_eq!(marked(&harness), Vec::<usize>::new());
}

#[test]
fn the_reader_mutes_the_open_conversation_says_so_and_m_unmutes_it() {
    let (mut harness, _dir, store) = open();
    click_row(&mut harness, 3, Modifiers::empty());
    assert_eq!(harness.count(".reader-head .muted-note"), 0);

    let tool = "[*|aria-label=\"Mute this conversation\"]";
    harness.click(centre(&harness, tool));
    harness.advance(ms(600));
    assert_eq!(muted(&store), vec![INBOX[2].1.to_owned()]);
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
    assert_eq!(harness.count(".reader-head .muted-note"), 0);

    // Each was its own gesture: Ctrl Z takes back the unmute, then the mute.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), vec![INBOX[2].1.to_owned()]);
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new());
}

#[test]
fn m_on_a_selection_mutes_every_picked_conversation_and_one_undo_takes_all_back() {
    let (mut harness, _dir, store) = open();
    click_row(&mut harness, 1, Modifiers::empty());
    click_row(&mut harness, 3, Modifiers::SHIFT);
    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    let three: Vec<String> = INBOX[..3].iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(muted(&store), three);
    assert_eq!(marked(&harness), vec![1, 2, 3]);
    assert!(
        toast(&harness).contains("Muted · 3 conversations"),
        "{}",
        toast(&harness)
    );

    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), Vec::<String>::new(), "one Ctrl Z, all three");
    assert_eq!(marked(&harness), Vec::<usize>::new());
}

#[test]
fn the_selection_bar_s_mute_mutes_the_rest_of_a_half_muted_selection() {
    let (mut harness, _dir, store) = open();
    // The second is muted first, on its own.
    click_row(&mut harness, 2, Modifiers::empty());
    harness.key(Key::Char('m'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), vec![INBOX[1].1.to_owned()]);

    click_row(&mut harness, 1, Modifiers::empty());
    click_row(&mut harness, 2, Modifiers::CONTROL);
    let button = "[*|aria-label=\"Mute the 2 selected\"]";
    harness.click(centre(&harness, button));
    harness.advance(ms(600));
    let two: Vec<String> = INBOX[..2].iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(muted(&store), two, "mutes the rest, never a toggle each");

    // Its undo takes back only what it did: the second stays muted.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(muted(&store), vec![INBOX[1].1.to_owned()]);
}
