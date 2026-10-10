//! The window on Blitz (`native`), driven the way its user drives it: pointer, keys and time,
//! against a real, headless Blitz document through `ds_harness::Harness`.
//!
//! Every case opens the real window (`mail_app::ui::native::root`, which is the launched window
//! less its watch on the settings directory) over a store seeded in a `TempDir`. The window is
//! handed no directories, so it writes no file anywhere; nothing here reads or touches the real
//! mail store or the real config.

use ds::prelude::*;
use ds_blitz::{FocusFallback, NetPolicy, PrintError, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

use crate::settle;
use mail_core::{Arrival, absorb};
use mail_core::{SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use settle::settle_until;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::drive;
use crate::row_menu;
use drive::{Drive, Key, PRIMARY};

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

/// How long quire waits for `token`.
fn delay(token: ds::style::tokens::delay::DelayToken) -> Duration {
    token.delay()
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
        // Newest first: each an hour older than the one before it.
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <seed{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
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
    launch_at(answer, seed, fallback, |_| mail_app::ui::Start::Inbox)
}

/// [`launch`], opened where `start` says once the store is seeded, as `main` decides it.
fn launch_at(
    answer: fn() -> Result<PrintOutcome, PrintError>,
    seed: fn(&SqliteStore),
    fallback: FocusFallback,
    start: impl FnOnce(&SqliteStore) -> mail_app::ui::Start,
) -> (Harness, tempfile::TempDir, Arc<SqliteStore>, Printed) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    seed(&store);
    let start = start(&store);
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
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        start,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(fallback)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    // quire's entrances run on a frame's wait after mount.
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    (harness, dir, store, printed)
}

/// The subjects of the list's rows, top to bottom, as the document holds them.
fn subjects(harness: &Harness) -> Vec<String> {
    const SUBJECT: &str = "class=\"ds-thread-sub ds-truncate\">";
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
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread")
}

/// Click the `n`th row where a person reads it: the start of its subject line. The row's hover
/// strip (Archive first) is laid out over the right half of the row from its middle down, and
/// a click at the row's very centre lands on it.
fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!("{} .ds-thread-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
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
    assert_eq!(harness.count(".reader .ds-empty-state"), 1);
    assert!(
        harness.is_focused(".app"),
        "the window does not hold the keyboard"
    );
}

/// `mailo mailto:…`, as the desktop runs the scheme's handler: the window opens on a composer,
/// laid out, holding what the link asked for, and nothing is sent.
#[test]
fn a_window_started_from_a_mailto_link_opens_on_the_composer() {
    let uri = "mailto:ada@example.test?subject=About%20the%20flight&body=Which%20gate%3F";
    let link = mail_app::ui::mailto_of(&[uri.to_owned()]).expect("read as a mailto link");
    let (harness, _dir, store, _printed) = launch_at(
        || Ok(PrintOutcome::Cancelled),
        |_| {},
        FocusFallback::Ancestor,
        |store| {
            mail_app::ui::start_mailto(store, &link, chrono::Utc::now())
                .expect("the link's draft is saved")
        },
    );

    let subject = ".c-title input";
    let rect = harness
        .rect(subject)
        .unwrap_or_else(|| panic!("no subject field is drawn:\n{}", harness.html()));
    assert!(
        rect.size.width.0 > 100.0,
        "the subject field is not laid out: {rect:?}"
    );
    let html = harness.html();
    assert!(
        html.contains(r#"value="About the flight""#),
        "not the link's subject:\n{html}"
    );
    assert!(html.contains("Which gate?"), "not the link's body:\n{html}");
    assert!(
        html.contains(r#"aria-label="Remove ada""#),
        "not the link's recipient:\n{html}"
    );

    // A draft, held and not queued: a link can open a composer, never send.
    let drafts = store.drafts(acct_account()).unwrap();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert_eq!(drafts[0].state, SendState::Editing);
    let a_year_on = chrono::Utc::now() + chrono::Duration::days(365);
    assert!(
        store
            .outbox_due(acct_account(), a_year_on)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn clicking_a_row_opens_it_in_the_reader() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 2);
    assert_eq!(
        harness.count(".reader .ds-empty-state"),
        0,
        "the reader stayed empty"
    );
    let reader = harness.text_of(".reader").unwrap_or_default();
    assert!(
        reader.contains("The invoice for September"),
        "the reader does not show the clicked thread: {reader}"
    );
    let frame = harness
        .frame("article.frame iframe.html")
        .expect("the frame has a document");
    assert!(
        frame
            .html()
            .contains("The body of The invoice for September."),
        "the frame does not show the body: {}",
        frame.html()
    );
    assert_eq!(
        harness
            .attr(
                ".list .ds-list-item[*|aria-posinset=\"2\"] .ds-row",
                "aria-selected"
            )
            .as_deref(),
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
    let hold = delay(ds::style::tokens::delay::DelayToken::ToastHold);
    let asked = harness.now();
    harness.key(Key::Char('e'));
    harness.advance(ms(300));
    assert_eq!(harness.count(".ds-toast"), 1, "archiving put up no toast");
    assert_ne!(
        harness.attr(".ds-toast", "data-presence").as_deref(),
        Some("leaving"),
        "the toast is already going"
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

/// The search panel, three ways in. Cmd K puts the keyboard in it, typed letters are its query
/// and not shortcuts, the first Escape empties the field and the second gives the keyboard back.
/// The toolbar's magnifier brings it up too, and what is typed filters the rows. Ctrl+F brings it
/// up over an open conversation.
#[test]
fn the_search_panel_takes_the_keyboard_filters_the_rows_and_gives_the_keyboard_back() {
    let (mut harness, _dir) = open();

    // Cmd K, then Escape twice.
    assert_eq!(harness.count(".ds-menu"), 0);
    harness.chord(&[PRIMARY], Key::Char('k'));
    harness.advance(ms(300));
    assert!(
        harness.is_focused(".ds-search-card input"),
        "Cmd K (Ctrl under Toshy) left the keyboard elsewhere"
    );
    // The search panel is up at the top of the window; there is no palette over it.
    settle_until(&mut harness, |h| h.count(".ds-search-card .ds-menu") == 1);
    assert_eq!(harness.count(".ds-palette"), 0, "Cmd K: a palette is drawn");
    // Typed letters are the panel's, not shortcuts: the list's search, and the panel's query.
    for key in "arch".chars() {
        harness.key(Key::Char(key));
    }
    harness.advance(ms(200));
    assert_eq!(
        harness.attr(".ds-search-card input", "value").as_deref(),
        Some("arch"),
        "Cmd K: the typed query"
    );
    // The first Escape empties the field and keeps the keyboard there.
    harness.key(Key::Escape);
    harness.advance(ms(300));
    assert_eq!(
        harness.attr(".ds-search-card input", "value").as_deref(),
        Some(""),
        "the first Escape did not empty the field"
    );
    assert!(
        harness.is_focused(".ds-search-card input"),
        "the first Escape left the field"
    );
    settle_until(&mut harness, |h| subjects(h).len() == INBOX.len());
    // The second leaves it, and the panel goes with it.
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(
        harness.count(".ds-search-card"),
        0,
        "Escape left the panel open"
    );
    assert!(harness.is_focused(".app"), "the keyboard did not come back");

    // The magnifier, and typing that filters the rows.
    harness.click(centre(&harness, SEARCH_BUTTON));
    settle_until(&mut harness, |h| h.is_focused(".ds-search-card input"));
    for key in "invoice".chars() {
        harness.key(Key::Char(key));
    }
    // The search waits for the box to be still (150 ms), then asks off the thread: wait for the
    // rows it brings back.
    settle_until(&mut harness, |h| {
        subjects(h) == vec!["The invoice for September".to_owned()]
    });
    assert_eq!(
        harness.attr(".ds-search-card input", "value").as_deref(),
        Some("invoice"),
        "the magnifier: the typed query"
    );
    // Out again, the rows back.
    harness.key(Key::Escape);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| subjects(h).len() == INBOX.len());
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(
        harness.count(".ds-search-card"),
        0,
        "Escape left the magnifier's panel open"
    );

    // Ctrl+F over an open conversation.
    open_row(&mut harness, 1);
    harness.chord(&[PRIMARY], Key::Char('f'));
    harness.advance(ms(300));
    assert!(
        harness.is_focused(".ds-search-card input"),
        "Ctrl+F left the keyboard elsewhere"
    );
}

/// The toolbar's search: the magnifier while nothing is searched.
const SEARCH_BUTTON: &str = ".list-head .bar [*|aria-label=\"Search\"]";

// Beyond the eight: the native host's own asks, and what is not on Blitz yet.

// The keyboard when what had it goes away. quire's `FocusFallback::Ancestor` (its "mailo gaps
// 7") gives it to the opener or the nearest focusable ancestor when the focused element is
// removed, and a quire control that keeps its click to itself takes the keyboard (v0.1.11).
// mailo keeps no re-focus of its own for either.

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
        .set_account_plan(acct_account(), &preset.plan)
        .unwrap();
    store
        .put_caps(acct_account(), &preset.expected_caps, chrono::Utc::now())
        .unwrap();
    let folder = |path: &str| Folder {
        account: acct_account(),
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(
            acct_account(),
            vec![folder("INBOX"), folder(PROJECTS), folder("Receipts")],
        )
        .unwrap();
}

/// The field a folder's rename is written in, in its name's place.
const RENAMING: &str = ".ds-row input[*|aria-label=\"Folder name\"]";

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
    // A fixed wait lands before select-all on a loaded runner: the field is focused and the
    // value is the old name, but the selection is still collapsed, so the next key would not
    // replace it. Wait for the selection itself.
    settle_until(&mut harness, |harness| {
        harness.count(".ds-menu") == 0
            && harness.is_focused(RENAMING)
            && harness.selected_text(RENAMING).as_deref() == Some(PROJECTS)
    });
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
        .folders(acct_account())
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
        .folders(acct_account())
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

/// With quire's fallback, `e` after the rename field went still archives the open conversation.
/// With it turned off, `e` is heard by nothing: nothing of mailo's puts the keyboard back after a
/// removal, so the first row passes on quire's fallback alone.
#[test]
fn when_the_focused_field_is_removed_the_keys_act_only_by_quire_s_fallback() {
    const CASES: [(&str, FocusFallback, &[&str]); 2] = [
        (
            "quire's fallback: `e` after the rename field went archived nothing, so the keyboard went nowhere",
            FocusFallback::Ancestor,
            &[INBOX[1].1, INBOX[2].1, INBOX[3].1],
        ),
        (
            "fallback off: `e` acted, so something of mailo's puts the keyboard back",
            FocusFallback::BlitzDefault,
            &[INBOX[0].1, INBOX[1].1, INBOX[2].1, INBOX[3].1],
        ),
    ];
    for (case, fallback, left) in CASES {
        let (harness, _dir) = escape_the_rename_then_press_e(fallback);
        assert_eq!(subjects(&harness), left, "{case}");
    }
}

/// The third row archived from its menu: the menu hands the keyboard back as it closes, but
/// the row is leaving, so the window takes it. Then `e` still archives the open conversation.
/// Every wait is for a state, not a time: this is the test that failed on a loaded machine
/// (F172), when the row's own strip button was pressed.
#[test]
fn archiving_a_row_from_its_menu_leaves_the_keyboard_working() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    row_menu::row_action(&mut harness, &row(3), "Archive");
    // The keyboard must land on the window, never in the leaving row: from there it is lost
    // whenever the focus and the row's removal share a frame, which a loaded machine makes
    // likely (F172).
    settle_until(&mut harness, |harness| {
        harness.count(":focus") > 0 && !harness.is_focused("html")
    });
    assert!(
        harness.is_focused(".app"),
        "the menu's Archive left the keyboard in its own row, not on the window:\n{}",
        harness.html()
    );
    settle_until(&mut harness, |harness| {
        subjects(harness) == [INBOX[0].1, INBOX[1].1, INBOX[3].1]
    });
    harness.key(Key::Char('e'));
    settle_until(&mut harness, |harness| {
        subjects(harness) == [INBOX[1].1, INBOX[3].1]
    });
    assert!(
        harness.is_focused(".app"),
        "`e` left the window without the keyboard"
    );
}

/// ⌘P on the first row's conversation, and the toast that says how it ended. The PDF is made
/// on a blocking thread, so this waits (with time passing) for the toast rather than a frame.
fn print_first_row(harness: &mut Harness) -> String {
    open_row(harness, 1);
    harness.chord(&[PRIMARY], Key::Char('p'));
    settle_until(harness, |h| h.text_of(".ds-toast-body").is_some());
    harness.text_of(".ds-toast-body").unwrap_or_default()
}

/// ⌘P hands the open conversation to the print dialog as a PDF, once, titled with its subject,
/// and the toast says how the dialog ended: cancelled, or, with no dialog, where the PDF opened.
#[test]
fn ctrl_p_hands_the_printout_over_and_says_how_it_ended() {
    type Answer = fn() -> Result<PrintOutcome, PrintError>;
    const CASES: [(&str, Answer, &str); 2] = [
        (
            "cancelled in the dialog",
            || Ok(PrintOutcome::Cancelled),
            "Printing cancelled",
        ),
        (
            "opened in a PDF viewer",
            || {
                Ok(PrintOutcome::Opened(std::path::PathBuf::from(
                    "/tmp/Flight-to-the-conference-1.pdf",
                )))
            },
            "Opened in your PDF viewer to print: /tmp/Flight-to-the-conference-1.pdf",
        ),
    ];
    for (case, answer, says) in CASES {
        let (mut harness, _dir, _store, printed) = open_printing(answer);
        let said = print_first_row(&mut harness);
        assert_eq!(said, says, "{case}");
        let printed = printed.lock().unwrap().clone();
        assert_eq!(
            printed.len(),
            1,
            "{case}: the dialog was not asked once: {printed:?}"
        );
        assert_eq!(printed[0].1, INBOX[0].1, "{case}: the printout's title");
    }
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

/// Select the whole body and delete it, so the next step of a test starts from one empty
/// paragraph.
fn clear(harness: &mut Harness, before: &str) {
    press(harness, &[PRIMARY], Key::Char('a'), 1);
    press(harness, &[], Key::Backspace, 1);
    harness.advance(ms(100));
    assert_eq!(
        paragraphs(harness),
        vec![""],
        "the body did not empty before {before}"
    );
}

/// The body's keys, each step on an emptied body: Enter splits a paragraph, Shift+Enter breaks
/// a line and Backspace merges; Up and Down move between laid-out lines; Home and End go to the
/// line's ends and Delete takes the next letter.
#[test]
fn the_body_s_keys_split_merge_move_and_delete() {
    let (mut harness, _dir, _store) = composing();

    // Enter, Shift+Enter, Backspace.
    type_text(&mut harness, "one");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "two");
    press(&mut harness, &[Key::Shift], Key::Enter, 1);
    type_text(&mut harness, "three");
    assert_eq!(
        paragraphs(&harness),
        vec!["one", "two\nthree"],
        "Enter split or Shift+Enter broke wrongly"
    );
    // Back to the second paragraph's start, then Backspace joins it to the first.
    press(&mut harness, &[], Key::Left, "two\nthree".len());
    press(&mut harness, &[], Key::Backspace, 1);
    assert_eq!(
        paragraphs(&harness),
        vec!["onetwo\nthree"],
        "Backspace did not merge"
    );
    // The caret stayed at the join: typing lands there.
    type_text(&mut harness, "-");
    assert_eq!(
        paragraphs(&harness),
        vec!["one-two\nthree"],
        "the caret left the join"
    );

    // Up and Down.
    clear(&mut harness, "Up and Down");
    type_text(&mut harness, "first");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "x");
    // From the start of the second line, up is the start of the first.
    press(&mut harness, &[], Key::Left, 1);
    press(&mut harness, &[], Key::Up, 1);
    type_text(&mut harness, "A");
    assert_eq!(paragraphs(&harness), vec!["Afirst", "x"], "Up");
    // And down again, to the start of the second.
    press(&mut harness, &[], Key::Left, 1);
    press(&mut harness, &[], Key::Down, 1);
    type_text(&mut harness, "B");
    assert_eq!(paragraphs(&harness), vec!["Afirst", "Bx"], "Down");
    // Up from the first line is the document's start.
    press(&mut harness, &[], Key::Up, 2);
    type_text(&mut harness, "C");
    assert_eq!(
        paragraphs(&harness),
        vec!["CAfirst", "Bx"],
        "Up from the first line"
    );

    // Home, End and Delete.
    clear(&mut harness, "Home and End");
    type_text(&mut harness, "middle");
    press(&mut harness, &[], Key::Home, 1);
    type_text(&mut harness, "A");
    press(&mut harness, &[], Key::End, 1);
    type_text(&mut harness, "Z");
    assert_eq!(paragraphs(&harness), vec!["AmiddleZ"], "Home and End");
    // Delete, from the line's start, takes the letter after the caret.
    press(&mut harness, &[], Key::Home, 1);
    press(&mut harness, &[], Key::Delete, 1);
    assert_eq!(paragraphs(&harness), vec!["middleZ"], "Delete");
    // Shift+End selects to the line's end.
    press(&mut harness, &[Key::Shift], Key::End, 1);
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 1, "Shift+End drew no selection");
    press(&mut harness, &[], Key::Backspace, 1);
    assert_eq!(
        paragraphs(&harness),
        vec![""],
        "Backspace on the Shift+End selection"
    );
}

/// The caret and the selection from the pointer and the keys, each step on an emptied body: a
/// click puts the caret where it lands; Shift+click selects from the caret to where it lands; a
/// drag selects what it passes over; Shift+Left across a paragraph break selects both sides, and
/// Backspace deletes it as one. What is typed replaces a selection.
#[test]
fn clicks_drags_and_shifted_keys_place_the_caret_and_select() {
    let (mut harness, _dir, _store) = composing();

    // A click.
    type_text(&mut harness, "hello world");
    let (start, end) = run_ends(&harness);
    // Before the first letter.
    harness.click(start);
    harness.advance(ms(100));
    type_text(&mut harness, "X");
    assert_eq!(
        paragraphs(&harness),
        vec!["Xhello world"],
        "a click before the first letter"
    );
    // Past the end of the line.
    harness.click(end);
    harness.advance(ms(100));
    type_text(&mut harness, "!");
    assert_eq!(
        paragraphs(&harness),
        vec!["Xhello world!"],
        "a click past the end of the line"
    );

    // Shift+click.
    clear(&mut harness, "Shift+click");
    type_text(&mut harness, "hello world");
    let (start, end) = run_ends(&harness);
    harness.click(start);
    harness.advance(ms(100));
    assert_eq!(harness.count(".c-sel"), 0, "a plain click selected");
    harness.click_with(end, &[Key::Shift]);
    harness.advance(ms(100));
    assert!(
        harness.count(".c-sel") >= 1,
        "Shift+click drew no selection"
    );
    // The selection is the whole line: typing replaces it.
    type_text(&mut harness, "X");
    assert_eq!(
        paragraphs(&harness),
        vec!["X"],
        "typing over the Shift+click selection"
    );

    // A drag.
    clear(&mut harness, "the drag");
    type_text(&mut harness, "hello world");
    let (start, end) = run_ends(&harness);
    harness.drag(start, end, 6);
    harness.advance(ms(100));
    assert!(harness.count(".c-sel") >= 1, "the drag drew no selection");
    assert_eq!(harness.count(".c-caret"), 0, "a caret beside a selection");
    type_text(&mut harness, "Y");
    assert_eq!(
        paragraphs(&harness),
        vec!["Y"],
        "typing over the dragged selection"
    );

    // A selection across paragraphs.
    clear(&mut harness, "the selection across paragraphs");
    type_text(&mut harness, "one");
    press(&mut harness, &[], Key::Enter, 1);
    type_text(&mut harness, "two");
    // "e", the paragraph break and "two", selected backwards from the end.
    press(&mut harness, &[Key::Shift], Key::Left, 5);
    harness.advance(ms(100));
    assert!(
        harness.count(".c-sel") >= 2,
        "the selection across paragraphs is not drawn"
    );
    press(&mut harness, &[], Key::Backspace, 1);
    harness.advance(ms(100));
    assert_eq!(
        paragraphs(&harness),
        vec!["on"],
        "the selection across paragraphs was not deleted as one"
    );
    assert_eq!(
        harness.count(".c-sel"),
        0,
        "the selection outlived its text"
    );
}

/// What an IME commits is written, and nothing else: a zhuyin composition, its preedit drawn at
/// the caret and the document untouched until the commit; then, after typed text, a kana to
/// kanji conversion.
#[test]
fn an_ime_writes_its_commit_and_nothing_else() {
    let (mut harness, _dir, _store) = composing();

    // Zhuyin.
    harness.ime_start();
    harness.ime_update("ㄓ", "ㄓ".len());
    harness.ime_update("ㄓㄨ", "ㄓㄨ".len());
    harness.advance(ms(100));
    // The preedit is drawn at the caret, and the document has not been touched.
    assert_eq!(
        harness.text_of(".c-preedit").as_deref(),
        Some("ㄓㄨ"),
        "the zhuyin preedit"
    );
    assert_eq!(
        paragraphs(&harness),
        vec![""],
        "the preedit touched the body"
    );
    // A key while the IME composes is the IME's.
    harness.key(Key::Char('x'));
    // winit clears the preedit (an empty update), then commits.
    harness.ime_commit("注");
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["注"], "the zhuyin commit");
    assert_eq!(
        harness.count(".c-preedit"),
        0,
        "the preedit outlived the commit"
    );
    // Typing after it is text again, after the commit.
    type_text(&mut harness, "a");
    assert_eq!(paragraphs(&harness), vec!["注a"], "typing after the commit");

    // Kana to kanji, after typed text.
    type_text(&mut harness, " ");
    harness.ime_start();
    for preedit in ["に", "にほ", "にほん", "日本"] {
        harness.ime_update(preedit, preedit.len());
    }
    harness.advance(ms(100));
    assert_eq!(
        paragraphs(&harness),
        vec!["注a "],
        "the kana preedit touched the body"
    );
    harness.ime_commit("日本");
    harness.ime_end();
    harness.advance(ms(100));
    assert_eq!(paragraphs(&harness), vec!["注a 日本"], "the kanji commit");
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
    press(&mut harness, &[PRIMARY], Key::Char('a'), 1);
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
fn send_queues_the_typed_body_as_text_and_html() {
    let (mut harness, _dir, store) = composing();
    type_text(&mut harness, "hey there");
    harness.click(centre(&harness, ".c-props .c-pin input"));
    harness.advance(ms(100));
    type_text(&mut harness, "ada@example.test");
    press(&mut harness, &[], Key::Enter, 1);
    harness.click(centre(&harness, ".c-body"));
    harness.advance(ms(100));
    press(&mut harness, &[PRIMARY], Key::Enter, 1);
    harness.advance(ms(1500));
    let later = chrono::Utc::now() + chrono::Duration::days(1);
    let due = store.outbox_due(acct_account(), later).unwrap();
    assert_eq!(due.len(), 1, "nothing was queued");
    let ProtoOp::Submit { draft, rcpt_to, .. } = &due[0].op else {
        panic!("expected a submission, got {:?}", due[0].op);
    };
    assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
    let sent = store.draft(*draft).unwrap();
    assert_eq!(sent.text.trim_end(), "hey there");
}

/// The run's start and end on its line, and the line's middle: where a test presses.
fn run_ends(harness: &Harness) -> (Point, Point) {
    let text = harness.rect(".c-body > p span").expect("the run");
    let middle = text.origin.y.0 + text.size.height.0 / 2.0;
    (
        Point {
            x: Px(text.origin.x.0 + 1.0),
            y: Px(middle),
        },
        Point {
            x: Px(text.origin.x.0 + text.size.width.0 + 40.0),
            y: Px(middle),
        },
    )
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
    press(&mut harness, &[PRIMARY], Key::Char('b'), 1);
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
    // The field keeps the keyboard through a press on another folder's name (quire v0.1.11, and
    // still in v0.2.0), so the rename ends because mailo ends it, not because the field lost focus.
    // While Projects is being renamed, its name is the field; Receipts' row is the other
    // folder.
    let other = ".ds-row[*|data-place=\"Receipts\"]";
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
        .folders(acct_account())
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(
        paths.contains(&PROJECTS.to_owned()),
        "the abandoned rename was written: {paths:?}"
    );
}

/// Every tipped control in the window, the pointer rested on it past the tip delay, shows its
/// tip through quire's hover hub: the buttons whose tip is their name, those whose short tip
/// (a name and its key, "Print  ⌘P") is their description, and the message's time, whose tip
/// is the full date. Then a Space dot's tip names the Space and the key that switches to it,
/// tersely as every tip is (`Name  Key`).
#[test]
fn a_rested_pointer_shows_each_controls_tip() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 1);
    let html = harness.html();
    let mut tipped: Vec<(String, String)> = ["data-tip", "aria-description"]
        .into_iter()
        .flat_map(|attr| {
            html.split(&format!("{attr}=\""))
                .skip(1)
                .filter_map(|after| after.split('"').next())
                .map(move |tip| (format!("[{attr}=\"{tip}\"]"), tip.to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();
    tipped.sort();
    tipped.dedup();
    assert!(tipped.len() >= 4, "too few tipped controls: {tipped:?}");
    assert!(
        tipped.iter().any(|(_, tip)| tip == "Print  \u{2318}P"),
        "Print's tip is not its name and key: {tipped:?}"
    );
    let full = harness
        .attr("time.msg-when", "aria-label")
        .expect("the message's time is drawn");
    tipped.push(("time.msg-when".to_owned(), full));
    for (selector, tip) in tipped {
        // Away first, long enough for the hub to cool, so each tip waits its own delay.
        harness.pointer_move(Point {
            x: Px(5.0),
            y: Px(790.0),
        });
        harness.advance(ms(2500));
        assert_eq!(
            harness.count(".ds-tooltip"),
            0,
            "a tip stands with the pointer away"
        );
        harness.pointer_move(centre(&harness, &selector));
        harness.advance(ms(1300));
        assert_eq!(
            harness.text_of(".ds-tooltip").as_deref(),
            Some(tip.as_str()),
            "{selector}"
        );
    }

    // The Space dot, rested on after the pointer has been away.
    harness.pointer_move(Point {
        x: Px(5.0),
        y: Px(790.0),
    });
    harness.advance(ms(2500));
    harness.pointer_move(centre(&harness, ".ds-space-dot"));
    harness.advance(ms(1300));
    let tip = harness.text_of(".ds-tooltip").unwrap_or_default();
    assert_eq!(tip, "Space 1  \u{2318}1", "the Space dot's tip");
}
