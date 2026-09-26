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
use ds_native::harness::settle_until;
use ds_native::{
    FocusFallback, Harness, HarnessConfig, NetPolicy, PrintError, PrintOutcome, Viewport,
};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// How long quire waits, at the default motion level, for `token`.
fn delay(token: ds::DelayToken) -> Duration {
    token.delay(ds::MotionLevel::Standard)
}

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
    let (harness, dir, _store) = open_with_store();
    (harness, dir)
}

/// [`open`], with the store the window writes, for a test to read back.
fn open_with_store() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let (harness, dir, store, _printed) = open_printing(|| Ok(PrintOutcome::Cancelled));
    (harness, dir, store)
}

/// What reached the print dialog: each PDF's length, and its title.
type Printed = Arc<Mutex<Vec<(usize, String)>>>;

/// [`open_with_store`], with a print dialog that records what it was given and answers
/// `answer`. Every window here has one: none of these tests may open the system's dialog.
fn open_printing(
    answer: fn() -> Result<PrintOutcome, PrintError>,
) -> (Harness, tempfile::TempDir, Arc<SqliteStore>, Printed) {
    launch(answer, |_| {}, FocusFallback::Ancestor)
}

/// The window over the seeded store, which `seed` adds to first, with a print dialog that
/// answers `answer`, and quire's `fallback` for a keyboard left nowhere.
fn launch(
    answer: fn() -> Result<PrintOutcome, PrintError>,
    seed: fn(&SqliteStore),
    fallback: FocusFallback,
) -> (Harness, tempfile::TempDir, Arc<SqliteStore>, Printed) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    seed(&store);
    let printed = Printed::default();
    let printer = mail_app::ui::native::Printer::with_dialog({
        let printed = Arc::clone(&printed);
        move |pdf, title| {
            printed.lock().unwrap().push((pdf.len(), title.to_owned()));
            answer()
        }
    });
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
        .with_focus_fallback(fallback)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    // quire's entrances run on a frame's wait after mount.
    harness.advance(ms(300));
    (harness, dir, store, printed)
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
    // quire hands the keyboard back to the menu's opener when the menu goes (its `HostHandBack`,
    // quire after v0.1.9, "mailo gaps 7"), rather than to `.app`.
    assert!(
        harness.is_focused(group),
        "the keyboard did not come back to the opener"
    );
}

#[test]
fn a_hover_card_opens_after_its_delay_and_not_before() {
    let (mut harness, _dir) = open();
    let sender = format!("{} .ds-row-name", row(1));
    let open_after = ds::delays::HOVER_OPEN;
    let asked = Instant::now();
    harness.pointer_move(centre(&harness, &sender));
    // `advance` lets wall-clock time pass, and hover intent sleeps on real timers, so under a
    // loaded machine a state asserted at one instant near the boundary flakes (quire's
    // CONVENTIONS §11). Assert the order instead: absent at half the delay, then present within
    // the settle bound, and never before the whole delay since the pointer arrived.
    harness.advance(open_after / 2);
    assert_eq!(harness.count(".ds-hovercard"), 0, "the card opened early");
    let opened = settle_until(&mut harness, |harness| harness.count(".ds-hovercard") == 1);
    assert!(
        opened.duration_since(asked) >= open_after,
        "the card opened {:?} after the pointer arrived, inside the {open_after:?} wait",
        opened.duration_since(asked)
    );
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
    let hold = delay(ds::DelayToken::ToastHold);
    let asked = Instant::now();
    harness.key(Key::Char('e'));
    harness.advance(ms(300));
    assert_eq!(
        harness.attr(".ds-toast", "data-shown").as_deref(),
        Some("shown"),
        "archiving put up no toast"
    );
    // Still up at half its hold (quire's CONVENTIONS §11: a "not yet" only at or under half the
    // window), then gone within the settle bound, and never before the whole hold.
    harness.advance(hold / 2 - ms(300));
    assert_eq!(
        harness.count(".ds-toast"),
        1,
        "the toast left before its hold"
    );
    harness.advance(hold / 2 - ms(1000));
    let gone = settle_until(&mut harness, |harness| harness.count(".ds-toast") == 0);
    assert!(
        gone.duration_since(asked) >= hold,
        "the toast left {:?} after the archive, inside its {hold:?} hold",
        gone.duration_since(asked)
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

// The keyboard when what had it goes away. quire's `FocusFallback::Ancestor` (its "mailo gaps
// 7") gives it to the opener or the nearest focusable ancestor when the focused element is
// removed, and mailo keeps no re-focus of its own for that. What quire does not reach, a click a
// quire component keeps to itself, is `Host::press_ended`'s (`ui/host/native.rs`).

/// A folder the seeded account holds: an IMAP account's, so the sidebar draws a Folders section.
const PROJECTS: &str = "Projects";

/// The seeded account as an IMAP account whose folders have been listed: the inbox and
/// [`PROJECTS`].
fn with_folders(store: &SqliteStore) {
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@example.test", &manual, chrono::Utc::now());
    store
        .connection()
        .execute(
            "UPDATE accounts SET plan = ?1 WHERE id = ?2",
            [
                serde_json::to_string(&preset.plan).unwrap(),
                ACCOUNT.to_string(),
            ],
        )
        .unwrap();
    store
        .put_caps(ACCOUNT, &preset.expected_caps, chrono::Utc::now())
        .unwrap();
    let folder = |path: &str| Folder {
        account: ACCOUNT,
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(
            ACCOUNT,
            vec![folder("INBOX"), folder(PROJECTS), folder("Receipts")],
        )
        .unwrap();
}

/// The field a folder's rename is written in, in its name's place.
const RENAMING: &str = "[*|data-slot=editing] input";

/// The window over [`with_folders`], the first conversation open, and [`PROJECTS`]' rename
/// begun from its ⋯ menu: the field in its name's place has the keyboard.
fn renaming(fallback: FocusFallback) -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let (mut harness, dir, store, _printed) =
        launch(|| Ok(PrintOutcome::Cancelled), with_folders, fallback);
    open_row(&mut harness, 1);
    let more = format!("[*|aria-label=\"Actions for {PROJECTS}\"]");
    // The ⋯ shows on the row's hover.
    harness.pointer_move(centre(&harness, &more));
    harness.advance(ms(300));
    harness.click(centre(&harness, &more));
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".ds-menu"),
        1,
        "the folder's menu did not open"
    );
    // New folder inside, then Rename.
    harness.key(Key::Down);
    harness.key(Key::Enter);
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 0, "the pick left the menu open");
    assert!(
        harness.is_focused(RENAMING),
        "the rename field does not have the keyboard:\n{}",
        harness.html()
    );
    (harness, dir, store)
}

#[test]
fn a_folder_is_renamed_in_its_name_s_place() {
    let (mut harness, _dir, store) = renaming(FocusFallback::Ancestor);
    // In the name's place: the name's own button is gone from the row, nothing is drawn under it.
    assert_eq!(harness.attr(RENAMING, "value").as_deref(), Some(PROJECTS));
    assert_eq!(
        harness.selected_text(RENAMING).as_deref(),
        Some(PROJECTS),
        "the old name is not selected"
    );
    assert_eq!(
        harness.count(".fold-new"),
        0,
        "a field is drawn under the row"
    );
    // Typed letters replace the selection and are not shortcuts (`s`, `a` and `p` are).
    for key in "Plans".chars() {
        harness.key(Key::Char(key));
    }
    harness.advance(ms(200));
    assert_eq!(harness.attr(RENAMING, "value").as_deref(), Some("Plans"));
    harness.key(Key::Enter);
    harness.advance(ms(600));
    assert_eq!(harness.count(RENAMING), 0, "Enter left the field");
    let paths: Vec<String> = store
        .folders(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(paths.contains(&"Plans".to_owned()), "{paths:?}");
    assert!(!paths.contains(&PROJECTS.to_owned()), "{paths:?}");
    assert_eq!(
        subjects(&harness).len(),
        INBOX.len(),
        "a letter was a shortcut"
    );
}

/// Escape leaves the rename: the field, which had the keyboard, is removed with it. Then `e`,
/// heard by the window's key handler, archives the open conversation, or, with the keyboard gone
/// nowhere, is heard by nothing.
fn escape_the_rename_then_press_e(fallback: FocusFallback) -> (Harness, tempfile::TempDir) {
    let (mut harness, dir, store) = renaming(fallback);
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(harness.count(RENAMING), 0, "Escape left the field");
    let paths: Vec<String> = store
        .folders(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(
        paths.contains(&PROJECTS.to_owned()),
        "Escape renamed: {paths:?}"
    );
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    (harness, dir)
}

#[test]
fn when_the_focused_field_is_removed_the_keys_still_act() {
    let (harness, _dir) = escape_the_rename_then_press_e(FocusFallback::Ancestor);
    assert_eq!(
        subjects(&harness),
        [INBOX[1].1, INBOX[2].1, INBOX[3].1],
        "`e` after the rename field went archived nothing: the keyboard went nowhere"
    );
}

/// The case above with quire's fallback turned off: `e` is heard by nothing. Nothing of
/// mailo's puts the keyboard back after a removal, so the case above passes on quire's alone.
#[test]
fn without_quire_s_fallback_a_removal_leaves_the_keyboard_nowhere() {
    let (harness, _dir) = escape_the_rename_then_press_e(FocusFallback::BlitzDefault);
    assert_eq!(
        subjects(&harness).len(),
        INBOX.len(),
        "`e` acted with the fallback off: something of mailo's puts the keyboard back"
    );
}

/// The third row's own Archive button pressed: quire's `HoverStrip` keeps the click to itself,
/// so it never reaches quire's click-focus fallback, and Blitz leaves the keyboard nowhere. Then
/// `e` still archives the open conversation (`Host::press_ended`).
#[test]
fn a_press_on_a_row_s_strip_leaves_the_keyboard_working() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    let third = format!("{} .ds-row-sub", row(3));
    harness.pointer_move(centre(&harness, &third));
    harness.advance(ms(300));
    let archive = format!("{} .ds-strip [*|data-op=archive]", row(3));
    harness.click(centre(&harness, &archive));
    harness.advance(ms(1500));
    assert_eq!(
        subjects(&harness),
        [INBOX[0].1, INBOX[1].1, INBOX[3].1],
        "the strip's Archive did not archive its row"
    );
    harness.key(Key::Char('e'));
    harness.advance(ms(1500));
    assert_eq!(
        subjects(&harness),
        [INBOX[1].1, INBOX[3].1],
        "`e` after the strip's Archive archived nothing: the keyboard went nowhere"
    );
}

/// Ctrl P on the first row's conversation, and the toast that says how it ended. The PDF is made
/// on a blocking thread, so this waits (with time passing) for the toast rather than a frame.
fn print_first_row(harness: &mut Harness) -> String {
    open_row(harness, 1);
    harness.chord(&[Key::Ctrl], Key::Char('p'));
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(120) {
        harness.advance(ms(50));
        if let Some(said) = harness.text_of(".ds-toast-text") {
            return said;
        }
    }
    panic!("Ctrl P put up no toast:\n{}", harness.html());
}

#[test]
fn print_hands_the_conversation_to_the_print_dialog_as_a_pdf() {
    let (mut harness, _dir, _store, printed) = open_printing(|| Ok(PrintOutcome::Cancelled));
    let said = print_first_row(&mut harness);
    assert_eq!(said, "Printing cancelled; nothing was printed.");
    let printed = printed.lock().unwrap().clone();
    assert_eq!(
        printed.len(),
        1,
        "the dialog was not asked once: {printed:?}"
    );
    let (len, title) = &printed[0];
    assert_eq!(title, INBOX[0].1);
    assert!(*len > 1_000, "a {len}-byte printout");
}

#[test]
fn print_without_a_dialog_says_where_the_pdf_opened() {
    let (mut harness, _dir, _store, printed) = open_printing(|| {
        Ok(PrintOutcome::Opened(std::path::PathBuf::from(
            "/tmp/Flight-to-the-conference-1.pdf",
        )))
    });
    let said = print_first_row(&mut harness);
    assert_eq!(
        said,
        "There is no print dialog here, so the printout opened in your PDF viewer to print \
         from there: /tmp/Flight-to-the-conference-1.pdf"
    );
    assert_eq!(printed.lock().unwrap().len(), 1);
}

#[test]
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

// The composer on quire's EditSurface (Phase B step 10): the editor core, driven through the
// adapter by real keys, IME events, pastes and clicks.

/// A new message open, its body clicked into so the surface has the keyboard.
fn composing() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let (mut harness, dir, store) = open_with_store();
    harness.key(Key::Char('c'));
    harness.advance(ms(500));
    harness.click(centre(&harness, ".c-body"));
    harness.advance(ms(200));
    (harness, dir, store)
}

/// Type `text` a key at a time, a space as the space bar.
fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    harness.advance(ms(100));
}

/// Press `key` `times` times with `held` down.
fn press(harness: &mut Harness, held: &[Key], key: Key, times: usize) {
    for _ in 0..times {
        harness.chord(held, key);
    }
    harness.advance(ms(100));
}

/// The text of each paragraph of the body, top to bottom.
fn paragraphs(harness: &Harness) -> Vec<String> {
    (1..=harness.count(".c-body > p"))
        .map(|n| {
            harness
                .text_of(&format!(".c-body > p:nth-child({n})"))
                .unwrap_or_default()
        })
        .collect()
}

fn near(a: f32, b: f32, by: f32) -> bool {
    (a - b).abs() <= by
}

#[test]
fn enter_splits_shift_enter_breaks_and_backspace_merges() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "one");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "two");
    press(&mut harness, &[Key::Shift], Key::Enter, 1);
    type_text(&mut harness, "three");
    assert_eq!(paragraphs(&harness), vec!["one", "two\nthree"]);
    // Back to the second paragraph's start, then Backspace joins it to the first.
    press(&mut harness, &[], Key::Left, "two\nthree".len());
    press(&mut harness, &[], Key::Backspace, 1);
    assert_eq!(paragraphs(&harness), vec!["onetwo\nthree"]);
    // The caret stayed at the join: typing lands there.
    type_text(&mut harness, "-");
    assert_eq!(paragraphs(&harness), vec!["one-two\nthree"]);
}

#[test]
fn up_and_down_move_between_laid_out_lines() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "first");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "x");
    // From the start of the second line, up is the start of the first.
    press(&mut harness, &[], Key::Left, 1);
    press(&mut harness, &[], Key::Up, 1);
    type_text(&mut harness, "A");
    assert_eq!(paragraphs(&harness), vec!["Afirst", "x"]);
    // And down again, to the start of the second.
    press(&mut harness, &[], Key::Left, 1);
    press(&mut harness, &[], Key::Down, 1);
    type_text(&mut harness, "B");
    assert_eq!(paragraphs(&harness), vec!["Afirst", "Bx"]);
    // Up from the first line is the document's start.
    press(&mut harness, &[], Key::Up, 2);
    type_text(&mut harness, "C");
    assert_eq!(paragraphs(&harness), vec!["CAfirst", "Bx"]);
}

#[test]
fn a_selection_across_paragraphs_is_deleted_as_one() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "one");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "two");
    // "e", the paragraph break and "two", selected backwards from the end.
    press(&mut harness, &[Key::Shift], Key::Left, 5);
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 2, "the selection is not drawn");
    press(&mut harness, &[], Key::Backspace, 1);
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["on"]);
    assert_eq!(
        harness.count(".c-sel"),
        0,
        "the selection outlived its text"
    );
}

#[test]
fn a_zhuyin_composition_writes_its_commit_and_nothing_else() {
    let (mut harness, _dir, _store) = composing();
    harness.ime_start();
    harness.ime_update("ㄓ", "ㄓ".len());
    harness.ime_update("ㄓㄨ", "ㄓㄨ".len());
    harness.advance(ms(100));
    // The preedit is drawn at the caret, and the document has not been touched.
    assert_eq!(harness.text_of(".c-preedit").as_deref(), Some("ㄓㄨ"));
    assert_eq!(paragraphs(&harness), vec![""]);
    // A key while the IME composes is the IME's.
    harness.key(Key::Char('x'));
    // winit clears the preedit (an empty update), then commits.
    harness.ime_commit("注");
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["注"]);
    assert_eq!(harness.count(".c-preedit"), 0);
    // Typing after it is text again, after the commit.
    type_text(&mut harness, "a");
    assert_eq!(paragraphs(&harness), vec!["注a"]);
}

#[test]
fn a_kana_to_kanji_conversion_commits_the_kanji() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "at ");
    harness.ime_start();
    for preedit in ["に", "にほ", "にほん", "日本"] {
        harness.ime_update(preedit, preedit.len());
    }
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["at "]);
    harness.ime_commit("日本");
    harness.ime_end();
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["at 日本"]);
}

#[test]
fn ctrl_b_bolds_the_selection_and_ctrl_z_undoes_it() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "bold move");
    press(&mut harness, &[Key::Shift], Key::Left, 4);
    press(&mut harness, &[Key::Ctrl], Key::Char('b'), 1);
    assert_eq!(harness.text_of(".c-body .m-b").as_deref(), Some("move"));
    // Still selected, with the bubble over it.
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 1, "the selection went");
    assert_eq!(harness.count(".bubble"), 1, "no bubble over the selection");
    press(&mut harness, &[Key::Ctrl], Key::Char('z'), 1);
    assert_eq!(harness.count(".c-body .m-b"), 0, "undo left the bold");
    assert_eq!(paragraphs(&harness), vec!["bold move"]);
}

#[test]
fn pasted_html_is_sanitised_into_blocks() {
    let (mut harness, _dir, _store) = composing();
    harness.paste_html(
        "<h1>Agenda</h1><p onclick=\"steal()\">First <b>this</b></p><script>alert(1)</script>",
        "Agenda\nFirst this",
    );
    harness.advance(ms(200));
    assert_eq!(harness.text_of(".c-body h2").as_deref(), Some("Agenda"));
    assert_eq!(harness.text_of(".c-body .m-b").as_deref(), Some("this"));
    let body = harness.text_of(".c-body").unwrap_or_default();
    assert!(body.contains("First this"), "{body}");
    assert!(
        !body.contains("alert"),
        "a script's text came through: {body}"
    );
    assert!(!harness.html().contains("steal"), "a handler came through");
}

#[test]
fn a_click_puts_the_caret_where_it_lands() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "hello world");
    let text = harness.rect(".c-body > p span").expect("the run");
    let middle = text.origin.y.0 + text.size.height.0 / 2.0;
    // Before the first letter.
    harness.click(Point {
        x: ds::Px(text.origin.x.0 + 1.0),
        y: ds::Px(middle),
    });
    harness.advance(ms(100));
    type_text(&mut harness, "X");
    assert_eq!(paragraphs(&harness), vec!["Xhello world"]);
    // Past the end of the line.
    harness.click(Point {
        x: ds::Px(text.origin.x.0 + text.size.width.0 + 40.0),
        y: ds::Px(middle),
    });
    harness.advance(ms(100));
    type_text(&mut harness, "!");
    assert_eq!(paragraphs(&harness), vec!["Xhello world!"]);
}

#[test]
fn the_caret_and_the_selection_are_drawn_where_the_text_is() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "hey");
    harness.advance(ms(100));
    let text = harness.rect(".c-body > p span").expect("the run");
    let caret = harness.rect(".c-caret").expect("no caret is drawn");
    // After the last letter, on its line.
    assert!(
        near(caret.origin.x.0, text.origin.x.0 + text.size.width.0, 1.0),
        "caret {caret:?}, text {text:?}"
    );
    assert!(
        near(caret.origin.y.0, text.origin.y.0, 4.0),
        "caret {caret:?}, text {text:?}"
    );
    assert!(
        caret.size.height.0 >= text.size.height.0 - 4.0,
        "caret {caret:?}"
    );
    // Everything selected: one box over the run, and no caret.
    press(&mut harness, &[Key::Ctrl], Key::Char('a'), 1);
    harness.advance(ms(100));
    let boxed = harness.rect(".c-sel").expect("no selection is drawn");
    assert!(
        near(boxed.origin.x.0, text.origin.x.0, 1.0),
        "{boxed:?} {text:?}"
    );
    assert!(
        near(boxed.size.width.0, text.size.width.0, 1.0),
        "{boxed:?} {text:?}"
    );
    assert_eq!(harness.count(".c-caret"), 0, "a caret beside a selection");
    // The IME's candidate window follows the caret.
    press(&mut harness, &[], Key::Right, 1);
    harness.advance(ms(100));
    let caret = harness.rect(".c-caret").expect("the caret is back");
    let area = harness.ime_cursor_area().expect("the IME has no area");
    assert!(
        near(area.origin.x.0, caret.origin.x.0, 1.0),
        "{area:?} {caret:?}"
    );
    assert!(
        near(area.origin.y.0, caret.origin.y.0, 1.0),
        "{area:?} {caret:?}"
    );
}

#[test]
fn a_slash_opens_its_menu_at_the_caret() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "/");
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 1, "/ opened no menu");
    let caret = harness.rect(".c-caret").expect("the caret");
    let menu = harness.rect(".ds-menu").expect("the menu");
    let below = menu.origin.y.0 >= caret.origin.y.0 + caret.size.height.0 - 1.0;
    let above = menu.origin.y.0 + menu.size.height.0 <= caret.origin.y.0 + 1.0;
    assert!(
        below || above,
        "the menu covers the caret: {menu:?} {caret:?}"
    );
    assert!(
        near(menu.origin.x.0, caret.origin.x.0, 24.0),
        "the menu is not at the caret: {menu:?} {caret:?}"
    );
    // Its keys are the menu's while it is open, and Escape closes it without parking the draft.
    press(&mut harness, &[], Key::Escape, 1);
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 0, "Escape left the menu open");
    assert_eq!(
        harness.count(".cpage .c-body"),
        1,
        "Escape parked the draft"
    );
}

#[test]
fn an_at_sign_mentions_a_person_and_adds_them_to_cc() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "thanks @ada");
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 1, "@ opened no menu");
    press(&mut harness, &[], Key::Enter, 1);
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-menu"), 0);
    let cc = harness.text_of(".c-props").unwrap_or_default();
    assert!(cc.contains("ada"), "ada is not in Cc: {cc:?}");
    // The query became the person's name, and the caret is after it.
    type_text(&mut harness, "for");
    assert_eq!(paragraphs(&harness), vec!["thanks @ada for"]);
}

#[test]
fn send_queues_the_typed_body_as_text_and_html() {
    let (mut harness, _dir, store) = composing();
    type_text(&mut harness, "hey there");
    harness.click(centre(&harness, ".c-props .c-pin input"));
    harness.advance(ms(100));
    type_text(&mut harness, "ada@example.test");
    press(&mut harness, &[], Key::Enter, 1);
    harness.click(centre(&harness, ".c-body"));
    harness.advance(ms(100));
    press(&mut harness, &[Key::Ctrl], Key::Enter, 1);
    harness.advance(ms(1500));
    let later = chrono::Utc::now() + chrono::Duration::days(1);
    let due = store.outbox_due(ACCOUNT, later).unwrap();
    assert_eq!(due.len(), 1, "nothing was queued");
    let ProtoOp::Submit {
        draft,
        raw,
        rcpt_to,
        ..
    } = &due[0].op
    else {
        panic!("expected a submission, got {:?}", due[0].op);
    };
    assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
    let sent = store.draft(*draft).unwrap();
    assert_eq!(sent.text.trim_end(), "hey there");
    let html = sent.html.unwrap_or_default();
    assert!(html.contains("hey there"), "{html}");
    let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
    let bytes = String::from_utf8_lossy(&bytes);
    for part in ["text/plain", "text/html", "hey there"] {
        assert!(bytes.contains(part), "no {part} in:\n{bytes}");
    }
}

/// The run's start and end on its line, and the line's middle: where a test presses.
fn run_ends(harness: &Harness) -> (Point, Point) {
    let text = harness.rect(".c-body > p span").expect("the run");
    let middle = text.origin.y.0 + text.size.height.0 / 2.0;
    (
        Point {
            x: ds::Px(text.origin.x.0 + 1.0),
            y: ds::Px(middle),
        },
        Point {
            x: ds::Px(text.origin.x.0 + text.size.width.0 + 40.0),
            y: ds::Px(middle),
        },
    )
}

#[test]
fn shift_click_selects_from_the_caret_to_where_it_lands() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "hello world");
    let (start, end) = run_ends(&harness);
    harness.click(start);
    harness.advance(ms(100));
    assert_eq!(harness.count(".c-sel"), 0, "a plain click selected");
    harness.click_with(end, dioxus::prelude::Modifiers::SHIFT);
    harness.advance(ms(100));
    assert!(
        harness.count(".c-sel") >= 1,
        "Shift+click drew no selection"
    );
    // The selection is the whole line: typing replaces it.
    type_text(&mut harness, "X");
    assert_eq!(paragraphs(&harness), vec!["X"]);
}

#[test]
fn a_drag_selects_what_it_passes_over() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "hello world");
    let (start, end) = run_ends(&harness);
    harness.drag(start, end, 6);
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 1, "the drag drew no selection");
    assert_eq!(harness.count(".c-caret"), 0, "a caret beside a selection");
    type_text(&mut harness, "Y");
    assert_eq!(paragraphs(&harness), vec!["Y"]);
}

#[test]
fn home_and_end_go_to_the_line_s_ends_and_delete_takes_the_next_letter() {
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "middle");
    press(&mut harness, &[], Key::Home, 1);
    type_text(&mut harness, "A");
    press(&mut harness, &[], Key::End, 1);
    type_text(&mut harness, "Z");
    assert_eq!(paragraphs(&harness), vec!["AmiddleZ"]);
    // Delete, from the line's start, takes the letter after the caret.
    press(&mut harness, &[], Key::Home, 1);
    press(&mut harness, &[], Key::Delete, 1);
    assert_eq!(paragraphs(&harness), vec!["middleZ"]);
    // Shift+End selects to the line's end.
    press(&mut harness, &[Key::Shift], Key::End, 1);
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 1, "Shift+End drew no selection");
    press(&mut harness, &[], Key::Backspace, 1);
    assert_eq!(paragraphs(&harness), vec![""]);
}

/// A picture of the composer on Blitz, with text, a bold run, a selection and the bubble,
/// painted headlessly: for when no screen can be captured.
#[test]
#[ignore = "picture generator: set MAILO_SNAPSHOT to a .png path and run with --ignored"]
fn composer_snapshot() {
    let path = std::env::var("MAILO_SNAPSHOT").expect("MAILO_SNAPSHOT names the .png to write");
    let (mut harness, _dir, _store) = composing();
    type_text(&mut harness, "Here is where we landed");
    press(&mut harness, &[Key::Shift], Key::Left, 6);
    press(&mut harness, &[Key::Ctrl], Key::Char('b'), 1);
    press(&mut harness, &[], Key::Right, 1);
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "so nobody has to scroll back");
    press(&mut harness, &[Key::Shift], Key::Left, 6);
    harness.advance(ms(600));
    harness.render().unwrap().save(&path).unwrap();
    // And the caret, beside the same text with nothing selected, as `<path>.caret.png`.
    press(&mut harness, &[], Key::Left, 1);
    press(&mut harness, &[], Key::Right, 3);
    harness.advance(ms(600));
    let caret = format!("{}.caret.png", path.trim_end_matches(".png"));
    harness.render().unwrap().save(caret).unwrap();
}

#[test]
fn going_to_another_folder_ends_a_rename() {
    let (mut harness, _dir, store) = renaming(FocusFallback::Ancestor);
    // The field keeps the keyboard through a press on another folder's name (quire v0.1.11), so
    // the rename ends because mailo ends it, not because the field lost focus.
    // While Projects is being renamed, its name is the field, so the one name button left is
    // Receipts'.
    let other = "button.ds-tree-item-label";
    assert!(
        harness.count(other) > 0,
        "no other folder to go to:\n{}",
        harness.html()
    );
    harness.click(centre(&harness, other));
    harness.advance(ms(300));
    assert_eq!(
        harness.count(RENAMING),
        0,
        "going to another folder left the rename open"
    );
    let paths: Vec<String> = store
        .folders(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(
        paths.contains(&PROJECTS.to_owned()),
        "the abandoned rename was written: {paths:?}"
    );
}
