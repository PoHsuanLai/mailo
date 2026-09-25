//! The window on Blitz (`native`), driven the way its user drives it: pointer, keys and time,
//! against a real, headless Blitz document through `ds_native::Harness`.
//!
//! Every case opens the real window (`mail_app::ui::native::root`, which is the launched window
//! less its watch on the settings directory) over a store seeded in a `TempDir`. The window is
//! handed no directories, so it writes no file anywhere; nothing here reads or touches the real
//! mail store or the real config.
//!
//! Run with `cargo test -p mail-app --no-default-features --features native`.

#![cfg(feature = "native")]

use ds::{Key, Point};
use ds_native::{Harness, HarnessConfig, NetPolicy, Viewport};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
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
///
/// The capabilities are written because `account add` always writes them, and a store without
/// them is a state the application cannot reach.
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
        // Newest first: each an hour older than the one before it.
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <seed{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("seed{n}"),
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

/// The window over a freshly seeded store, first frame drawn. The `TempDir` must outlive it.
fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    // quire's entrances run on a frame's wait after mount.
    harness.advance(ms(300));
    (harness, dir)
}

/// The subjects of the list's rows, top to bottom, as the document holds them.
fn subjects(harness: &Harness) -> Vec<String> {
    const SUBJECT: &str = "class=\"ds-row-sub ds-truncate\">";
    harness
        .html()
        .split(SUBJECT)
        .skip(1)
        .filter_map(|after| after.split("</div>").next())
        .map(text)
        .collect()
}

/// `html`'s text, without its tags: a subject's marked runs are spans.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The `n`th row of the list (1-based), as the element a click lands on.
fn row(n: usize) -> String {
    format!(".ds-list > .row:nth-child({n}) .ds-row")
}

/// Click the `n`th row where a person reads it: the start of its subject line. The row's hover
/// strip (Archive first) is laid out over the right half of the row from its middle down, and
/// a click at the row's very centre lands on it.
fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!("{} .ds-row-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::Px(rect.origin.x.0 + 24.0),
        y: ds::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn top(harness: &Harness, selector: &str) -> f32 {
    harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
        .origin
        .y
        .0
}

#[test]
fn the_window_opens_on_the_seeded_inbox() {
    let (harness, _dir) = open();
    let want: Vec<String> = INBOX.iter().map(|(_, s)| s.to_string()).collect();
    assert_eq!(subjects(&harness), want);
    for n in 1..=INBOX.len() {
        let rect = harness.rect(&row(n)).expect("a seeded row is drawn");
        assert!(
            rect.size.height.0 > 20.0 && rect.size.width.0 > 200.0,
            "row {n} is not laid out: {rect:?}"
        );
    }
    // Nothing open yet, and the keyboard is the window's from the first frame.
    assert_eq!(harness.count(".reader-empty"), 1);
    assert!(
        harness.is_focused(".app"),
        "the window does not hold the keyboard"
    );
}

#[test]
fn clicking_a_row_opens_it_in_the_reader() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 2);
    assert_eq!(harness.count(".reader-empty"), 0, "the reader stayed empty");
    let reader = harness.text_of(".reader").unwrap_or_default();
    assert!(
        reader.contains("The body of The invoice for September."),
        "the reader does not show the clicked thread: {reader}"
    );
    assert_eq!(
        harness.attr(&row(2), "aria-selected").as_deref(),
        Some("true")
    );
}

#[test]
fn a_menu_opens_on_click_and_closes_on_escape() {
    let (mut harness, _dir) = open();
    let group = ".bar-tools .ds-button:nth-child(1)";
    assert_eq!(harness.attr(group, "aria-label").as_deref(), Some("Group"));
    assert_eq!(harness.count(".ds-menu"), 0);
    harness.click(centre(&harness, group));
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 1, "the Group menu did not open");
    assert_eq!(
        harness.attr(group, "aria-expanded").as_deref(),
        Some("true")
    );
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(harness.count(".ds-menu"), 0, "Escape left the menu open");
    assert_eq!(
        harness.attr(group, "aria-expanded").as_deref(),
        Some("false")
    );
    assert!(harness.is_focused(".app"), "the keyboard did not come back");
}

#[test]
fn a_hover_card_opens_after_its_delay_and_not_before() {
    let (mut harness, _dir) = open();
    let sender = format!("{} .ds-row-name", row(1));
    harness.pointer_move(centre(&harness, &sender));
    harness.advance(ms(250));
    assert_eq!(harness.count(".ds-hovercard"), 0, "the card opened early");
    harness.advance(ms(400));
    assert_eq!(harness.count(".ds-hovercard"), 1, "the card never opened");
    let card = harness.text_of(".ds-hovercard").unwrap_or_default();
    assert!(card.contains("ada@example.test"), "{card}");
}

#[test]
fn archiving_a_row_makes_it_leave_and_the_rows_below_heal() {
    let (mut harness, _dir) = open();
    let second = top(&harness, &row(2));
    let third = top(&harness, &row(3));
    open_row(&mut harness, 2);
    assert_eq!(subjects(&harness).len(), INBOX.len(), "opening archived");
    // `e` is Archive, heard by the window's own key handler after the click.
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    assert_eq!(
        subjects(&harness),
        vec![
            "Flight to the conference".to_owned(),
            "Lunch on Thursday".to_owned(),
            "Notes from the review".to_owned(),
        ],
        "the archived row did not leave"
    );
    // The row that was third has healed up into the second place, and so on down.
    let healed = top(&harness, &row(2));
    assert!(
        (healed - second).abs() < 1.0,
        "the rows below did not heal: row 2 was at {second}, is now at {healed}"
    );
    let healed = top(&harness, &row(3));
    assert!(
        (healed - third).abs() < 1.0,
        "the rows below did not heal: row 3 was at {third}, is now at {healed}"
    );
}

#[test]
fn the_toast_hides_after_its_hold() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    assert_eq!(harness.count(".ds-toast"), 0);
    harness.key(Key::Char('e'));
    harness.advance(ms(300));
    assert_eq!(
        harness.attr(".ds-toast", "data-shown").as_deref(),
        Some("shown"),
        "archiving put up no toast"
    );
    // quire's hold is 5.2 s at the default motion level.
    harness.advance(ms(4000));
    assert_eq!(
        harness.count(".ds-toast"),
        1,
        "the toast left before its hold"
    );
    harness.advance(ms(2500));
    assert_eq!(
        harness.count(".ds-toast"),
        0,
        "the toast stayed past its hold"
    );
}

#[test]
fn ctrl_t_opens_the_palette_with_the_keyboard_in_its_field() {
    let (mut harness, _dir) = open();
    assert_eq!(harness.count(".ds-palette"), 0);
    harness.chord(&[Key::Ctrl], Key::Char('t'));
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-palette"), 1, "Ctrl T opened no palette");
    assert!(
        harness.is_focused(".ds-palette input"),
        "the palette's field does not hold the keyboard"
    );
    // Typed letters are the palette's, not shortcuts.
    for key in "arch".chars() {
        harness.key(Key::Char(key));
    }
    harness.advance(ms(200));
    assert_eq!(
        harness.attr(".ds-palette input", "value").as_deref(),
        Some("arch")
    );
    assert_eq!(subjects(&harness).len(), INBOX.len());
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(
        harness.count(".ds-palette"),
        0,
        "Escape left the palette open"
    );
    assert!(harness.is_focused(".app"), "the keyboard did not come back");
}

#[test]
fn typing_in_the_search_box_filters_the_rows() {
    let (mut harness, _dir) = open();
    harness.click(centre(&harness, ".search input"));
    for key in "invoice".chars() {
        harness.key(Key::Char(key));
    }
    // The search waits for the box to be still (150 ms), then asks off the thread.
    harness.advance(ms(800));
    assert_eq!(
        harness.attr(".search input", "value").as_deref(),
        Some("invoice")
    );
    assert_eq!(
        subjects(&harness),
        vec!["The invoice for September".to_owned()]
    );
}

// Beyond the eight: the native host's own asks, and what is not on Blitz yet.

#[test]
fn ctrl_f_puts_the_keyboard_in_the_find_field_and_escape_gives_it_back() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    harness.chord(&[Key::Ctrl], Key::Char('f'));
    harness.advance(ms(300));
    // Found by selector and focused in the document (`ui/host/native.rs`).
    assert!(
        harness.is_focused(".find input"),
        "Ctrl F left the keyboard elsewhere"
    );
    for key in "body".chars() {
        harness.key(Key::Char(key));
    }
    harness.advance(ms(300));
    assert_eq!(
        harness.attr(".find input", "value").as_deref(),
        Some("body")
    );
    assert_eq!(harness.count("mark.hit.now"), 1, "no current match");
    // The letters stayed in the field: none of them was a shortcut.
    assert_eq!(subjects(&harness).len(), INBOX.len());
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(harness.count(".find"), 0, "Escape left the find field open");
    assert!(
        harness.is_focused(".app"),
        "Escape did not give the keyboard back"
    );
}

#[test]
fn print_says_it_is_not_here_yet() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    harness.chord(&[Key::Ctrl], Key::Char('p'));
    harness.advance(ms(300));
    assert_eq!(
        harness.text_of(".ds-toast-text").as_deref(),
        Some("Printing is not available in this window yet; Save for printing works.")
    );
}

#[test]
#[ignore = "gap G2: the composer's body is a contenteditable driven by the webview's script \
            (compose/wire.rs GLUE); on Blitz it opens but cannot take the focus, and typed keys \
            go to .app, so nothing is written. Moving it onto quire's EditSurface is a later wave"]
fn typing_into_a_new_message_writes_it() {
    let (mut harness, _dir) = open();
    harness.key(Key::Char('c'));
    harness.advance(ms(500));
    assert_eq!(harness.count(".cpage .c-body"), 1, "c opened no composer");
    harness.click(centre(&harness, ".c-body"));
    harness.advance(ms(200));
    for key in "hey".chars() {
        harness.key(Key::Char(key));
    }
    harness.advance(ms(300));
    assert_eq!(harness.text_of(".c-body").as_deref(), Some("hey"));
}

/// A picture of the window on the seeded inbox, with a thread open, painted headlessly by
/// Blitz: for when no screen can be captured (a locked session, CI).
#[test]
#[ignore = "picture generator: set MAILO_SNAPSHOT to a .png path and run with --ignored"]
fn snapshot() {
    let path = std::env::var("MAILO_SNAPSHOT").expect("MAILO_SNAPSHOT names the .png to write");
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    harness.advance(ms(600));
    harness.render().unwrap().save(path).unwrap();
}
