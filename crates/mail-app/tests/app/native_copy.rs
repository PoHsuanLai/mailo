//! Copy, every way the window offers it, in the real window on Blitz (`ds_harness::Harness`):
//! text selected in a message's body (its own sealed document) and in a plain-text message,
//! in the composer's body, in a plain field (the subject, the search panel), and a link's
//! right-click Copy Link. Each case selects the way a person does, copies with Ctrl+C (and Super+C,
//! the Command key as a window reports it), and reads what landed on the harness's clipboard,
//! which is in memory, never the desktop's.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`, with an
//! `Original` whose fetcher and browser are recorders: nothing here touches the network, the
//! real store, the real config or the desktop's clipboard.

use ds::base::press::PointerButton;
use ds::prelude::*;
use ds_blitz::FocusFallback;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_app::ui::native::{Browse, Fetch, Original};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::drive;
use drive::{Drive, Key, PRIMARY};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c3"))
}

const VIEW: Viewport = Viewport {
    width: 1280,
    height: 900,
    scale_percent: 100,
};

const FRAME: &str = "article.frame iframe.html";
const HEADING: &str = "The autumn collection is here";
const LINK_TO: &str = "https://shop.example.test/autumn";

fn letter(date: &str) -> String {
    format!(
        "From: Shop <letters@shop.example.test>\r\nTo: me@example.test\r\n\
         Subject: The autumn letter\r\nDate: {date}\r\nMessage-ID: <autumn@shop.example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <h1>{HEADING}</h1><p><a href=\"{LINK_TO}\">See the collection</a></p>\r\n"
    )
}

fn plain(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Lunch\r\nDate: {date}\r\n\
         Message-ID: <lunch@example.test>\r\n\r\nThursday at noon suits me.\r\n"
    )
}

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
    }
    let now = chrono::Utc::now();
    let bodies: [fn(&str) -> String; 2] = [letter, plain];
    for (n, body) in bodies.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
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
                    uidl: format!("copy{n}"),
                },
                raw: body(&date).into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// Fetches nothing: these letters have no remote parts.
struct NoFetch;
impl Fetch for NoFetch {
    fn get(&self, _: String, _: Box<dyn FnOnce(Vec<u8>) + Send>) {}
}

/// Opens nothing, and keeps what it was asked to open.
#[derive(Clone, Default)]
struct Opened(Arc<std::sync::Mutex<Vec<String>>>);
impl Browse for Opened {
    fn open(&self, url: &str) {
        self.0.lock().unwrap().push(url.to_owned());
    }
}

/// The platform's command key, as Blitz's own text actions read it: ⌘ (Super) on a Mac,
/// Ctrl elsewhere.
#[cfg(target_os = "macos")]
const COMMAND: Key = Key::Super;
#[cfg(not(target_os = "macos"))]
const COMMAND: Key = Key::Ctrl;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The window as `launch` wires it (`FocusFallback::Ancestor`), over the seeded store.
fn open() -> (Harness, tempfile::TempDir) {
    open_browsing(Opened::default())
}

/// The same, a link opened from it going to `browse`.
fn open_browsing(browse: Opened) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let original = Original::new(Arc::new(NoFetch), Arc::new(browse));
    let contexts = mail_app::ui::native::contexts(
        seeded(dir.path()),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Wall)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts)
        .with_net(original.net())
        .with_frame_links(original.links())
        .with_contexts(original.contexts());
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    until(&mut harness, "the list", |h| {
        h.count(".list .ds-thread") > 0
    });
    (harness, dir)
}

fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{what} never came:\n{}",
            harness.html()
        );
        harness.advance(ms(50));
    }
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread .ds-thread-sub");
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

/// Drag across the whole first line of the frame's `selector`, from just inside its left edge
/// to well past its right.
fn select_in_frame(harness: &mut Harness, selector: &str) {
    let rect = {
        let frame = harness.frame(FRAME).expect("a frame document");
        let at = frame.centre(selector).expect("the element is drawn");
        let width = frame.width(selector).expect("the element is drawn");
        (at, width)
    };
    let (at, width) = rect;
    let from = Point {
        x: Px(at.x.0 - width.0 / 2.0 + 1.0),
        y: at.y,
    };
    let to = Point {
        x: Px(at.x.0 + width.0 / 2.0 - 1.0),
        y: at.y,
    };
    harness.drag(from, to, 8);
    harness.advance(ms(100));
}

fn copy_with(harness: &mut Harness, held: Key) {
    harness.chord(&[held], Key::Char('c'));
    harness.advance(ms(100));
}

fn copied(harness: &Harness) -> String {
    harness.clipboard_text().unwrap_or_default()
}

fn letter_open() -> (Harness, tempfile::TempDir) {
    let (mut harness, dir) = open();
    open_row(&mut harness, 1);
    until(&mut harness, "the letter's frame", |h| {
        h.frame(FRAME).is_some_and(|f| f.text().contains(HEADING))
    });
    (harness, dir)
}

/// Each copy below lands text other than what the clipboard held, so a chord that copied
/// nothing fails its step rather than passing on the step before's text.
///
/// (a) Text selected in an HTML message's body, its own sealed document, then Ctrl+C. Then a
/// plain-text message, whose body is a frame of its own as well. Then (a, e) the HTML message
/// again with Super+C: the Command key, as the window reports it since quire v0.2.31. Blitz's
/// own copy of a page's selection takes Super as well as Ctrl off macOS since quire v0.3.0's
/// Blitz.
#[test]
fn the_command_key_and_super_c_copy_a_messages_selection() {
    let (mut harness, _dir) = letter_open();

    // The HTML message, the command key.
    select_in_frame(&mut harness, "h1");
    copy_with(&mut harness, COMMAND);
    let got = copied(&harness);
    assert!(
        got.contains("autumn collection"),
        "the command key in the HTML message copied {got:?}"
    );

    // The plain message, the command key.
    open_row(&mut harness, 2);
    until(&mut harness, "the plain body", |h| {
        h.frame(FRAME)
            .is_some_and(|f| f.text().contains("Thursday at noon"))
    });
    // A plain body is a frame too, its text straight in its `body` (16 px of margin, 14 px
    // type): drag along its first line.
    let frame = harness.rect(FRAME).expect("the frame is drawn");
    let y = Px(frame.origin.y.0 + 16.0 + 10.0);
    let from = Point {
        x: Px(frame.origin.x.0 + 17.0),
        y,
    };
    let to = Point {
        x: Px(frame.origin.x.0 + 400.0),
        y,
    };
    harness.drag(from, to, 8);
    harness.advance(ms(100));
    copy_with(&mut harness, COMMAND);
    let got = copied(&harness);
    assert!(
        got.contains("Thursday"),
        "the command key in the plain message copied {got:?}"
    );

    // The HTML message again, Super+C.
    open_row(&mut harness, 1);
    until(&mut harness, "the letter's frame again", |h| {
        h.frame(FRAME).is_some_and(|f| f.text().contains(HEADING))
    });
    select_in_frame(&mut harness, "h1");
    copy_with(&mut harness, Key::Super);
    let got = copied(&harness);
    assert!(
        got.contains("autumn collection"),
        "Super+C in the HTML message copied {got:?}"
    );
}

/// (d) A link in the frame: a right click on it offers Open Link and Copy Link at the pointer,
/// as a browser does. quire reports the link the press was on just before the reader's own
/// `oncontextmenu` (`FrameLinks::with_context_menu`); the reader opens the menu with it.
fn right_click_link(harness: &mut Harness) {
    let link = harness
        .frame(FRAME)
        .and_then(|f| f.centre("a"))
        .expect("the link is drawn");
    harness.press(link, PointerButton::Secondary);
    until(harness, "the link's menu", |h| h.count(LINK_MENU) == 2);
}

const LINK_MENU: &str = ".ds-menu .ds-menu-item";
const OPEN_LINK: &str = ".ds-menu .ds-menu-item:nth-child(1)";
const COPY_LINK: &str = ".ds-menu .ds-menu-item:nth-child(2)";

/// A right click on a link: Copy Link copies where it goes, Open Link opens it as a click would,
/// and a right click off a link then offers no link, not the last link's menu either: what the
/// frames report is taken by the menu it opens.
#[test]
fn a_right_click_on_a_link_opens_or_copies_it_and_off_a_link_offers_no_link() {
    let opened = Opened::default();
    let (mut harness, _dir) = open_browsing(opened.clone());
    open_row(&mut harness, 1);
    until(&mut harness, "the letter's frame", |h| {
        h.frame(FRAME).is_some_and(|f| f.text().contains(HEADING))
    });

    // Copy Link.
    right_click_link(&mut harness);
    let copy = harness.text_of(COPY_LINK).unwrap_or_default();
    assert!(
        copy.contains("Copy Link"),
        "Copy Link is not offered: {copy:?}"
    );
    let at = centre(&harness, COPY_LINK);
    harness.click(at);
    // The picked row blinks before the menu closes and the pick lands, as a Mac menu's does.
    until(&mut harness, "the menu to close after Copy Link", |h| {
        h.count(LINK_MENU) == 0
    });
    assert_eq!(copied(&harness), LINK_TO, "Copy Link copied");
    assert!(
        opened.0.lock().unwrap().is_empty(),
        "Copy Link opened the link"
    );

    // Open Link.
    right_click_link(&mut harness);
    let open = harness.text_of(OPEN_LINK).unwrap_or_default();
    assert!(
        open.contains("Open Link"),
        "Open Link is not offered: {open:?}"
    );
    let at = centre(&harness, OPEN_LINK);
    harness.click(at);
    until(&mut harness, "the menu to close after Open Link", |h| {
        h.count(LINK_MENU) == 0
    });
    assert_eq!(
        *opened.0.lock().unwrap(),
        vec![LINK_TO.to_owned()],
        "Open Link opened"
    );

    // Off a link.
    right_click_link(&mut harness);
    harness.key(Key::Escape);
    until(&mut harness, "the menu to close on Escape", |h| {
        h.count(LINK_MENU) == 0
    });
    let heading = harness
        .frame(FRAME)
        .and_then(|f| f.centre("h1"))
        .expect("the heading is drawn");
    harness.press(heading, PointerButton::Secondary);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(LINK_MENU),
        0,
        "a right click off a link offered a link"
    );
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
}

/// A new message, its page at rest.
fn composing() -> (Harness, tempfile::TempDir) {
    let (mut harness, dir) = open();
    harness.key(Key::Char('c'));
    until(&mut harness, "the composer", |h| {
        h.count(".cpage .c-body") == 1
    });
    harness.advance(ms(600));
    (harness, dir)
}

/// Click `field` and wait for it to have the keyboard.
fn focus(harness: &mut Harness, field: &str) {
    let at = centre(harness, field);
    harness.click(at);
    until(harness, &format!("{field}'s keyboard"), |h| {
        h.is_focused(field)
    });
}

/// Select all of what has the keyboard with `held`+A.
fn select_all(harness: &mut Harness, held: Key) {
    harness.chord(&[held], Key::Char('a'));
    harness.advance(ms(100));
}

/// In a new message, each step copying text other than the step before's: (b) the composer's
/// body, typed, all selected, Ctrl+C; (c) a plain field, the subject, with the command key;
/// (b, e) the body again with Super+C; (c, e) the subject with Super+C, since Blitz's text input
/// takes its select-all, copy, cut and paste chords on Super as well as Ctrl off macOS since
/// quire v0.3.0's Blitz; (c) the To field, before its address becomes a chip.
#[test]
fn in_a_new_message_ctrl_c_and_super_c_copy_the_body_s_and_each_field_s_selection() {
    const BODY: &str = ".c-body";
    const SUBJECT: &str = ".c-title input";
    const TO: &str = ".c-props [*|data-row=to] input";
    let (mut harness, _dir) = composing();

    // The body, Ctrl+C.
    focus(&mut harness, BODY);
    type_text(&mut harness, "see you there");
    select_all(&mut harness, Key::Ctrl);
    copy_with(&mut harness, Key::Ctrl);
    assert_eq!(copied(&harness), "see you there", "Ctrl+C in the body");

    // The subject, the command key.
    focus(&mut harness, SUBJECT);
    type_text(&mut harness, "Lunch plans");
    select_all(&mut harness, COMMAND);
    assert_eq!(
        harness.selected_text(SUBJECT).as_deref(),
        Some("Lunch plans"),
        "the command key's select-all in the subject"
    );
    copy_with(&mut harness, COMMAND);
    assert_eq!(
        copied(&harness),
        "Lunch plans",
        "the command key in the subject"
    );

    // The body again, Super+C.
    focus(&mut harness, BODY);
    select_all(&mut harness, Key::Super);
    copy_with(&mut harness, Key::Super);
    assert_eq!(copied(&harness), "see you there", "Super+C in the body");

    // The subject again, Super+C.
    focus(&mut harness, SUBJECT);
    select_all(&mut harness, Key::Super);
    assert_eq!(
        harness.selected_text(SUBJECT).as_deref(),
        Some("Lunch plans"),
        "Super+A in the subject"
    );
    copy_with(&mut harness, Key::Super);
    assert_eq!(copied(&harness), "Lunch plans", "Super+C in the subject");

    // The To field, the command key.
    focus(&mut harness, TO);
    type_text(&mut harness, "ada@example.test");
    select_all(&mut harness, COMMAND);
    copy_with(&mut harness, COMMAND);
    assert_eq!(
        copied(&harness),
        "ada@example.test",
        "the command key in To"
    );
}

/// (c) The search panel's field: typed, all selected, Ctrl+C.
#[test]
fn the_command_key_copies_the_search_fields_selection() {
    let (mut harness, _dir) = open();
    harness.chord(&[PRIMARY], Key::Char('k'));
    until(&mut harness, "the search field", |h| {
        h.is_focused(".spotlight input")
    });
    type_text(&mut harness, "invoice");
    harness.chord(&[COMMAND], Key::Char('a'));
    harness.advance(ms(100));
    copy_with(&mut harness, COMMAND);
    assert_eq!(copied(&harness), "invoice");
}
