//! Peek, the one preview, in the real window: a calendar file, a contact card and a forwarded
//! message drawn by mailo's own readers, the left and right arrows moving between them, Space on
//! a tile that has the keyboard opening it, and Esc closing; and a link's destination card, from
//! the link's right-click Preview Link, honest or warning that the link's text names another site,
//! with Open Link (or Primary+Return) going there through the window's opener.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`, with an
//! `Original` whose fetcher and browser are recorders: nothing here touches the network.

use ds::base::press::PointerButton;
use ds::prelude::*;
use ds_blitz::FocusFallback;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query};
use mail_app::ui::copy;
use mail_app::ui::native::{Browse, Fetch, Original};
use mail_core::{Arrival, SqliteStore, absorb};
use mail_domain::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::native_preview::{
    PEEK, TWO_PAGES, VIEW, acct_account, carrying, item, ms, png, with_account,
};
use crate::drive::{Drive, Key, PRIMARY};

const FRAME: &str = "article.frame iframe.html";
const HONEST: &str = "https://docs.example.test/agenda";
const LYING: &str = "https://bank-com.attacker.test/login";

const ICS: &[u8] = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nMETHOD:PUBLISH\r\n\
BEGIN:VEVENT\r\nUID:review@example.test\r\nDTSTAMP:20261001T090000Z\r\n\
DTSTART:20261020T090000Z\r\nDTEND:20261020T100000Z\r\nSUMMARY:Design review\r\n\
LOCATION:Room 4\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const VCF: &[u8] =
    b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada Lovelace\r\nEMAIL:ada@example.test\r\nEND:VCARD\r\n";
const EML: &[u8] = b"From: Grace <grace@example.test>\r\nTo: ada@example.test\r\n\
Subject: Lunch on Friday\r\nDate: Tue, 13 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <lunch@example.test>\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nSee you at noon.\r\n";

fn letter(date: &str) -> String {
    format!(
        "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: Two links\r\n\
         Date: {date}\r\nMessage-ID: <links@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         <p><a id=\"honest\" href=\"{HONEST}\">the agenda</a></p>\
         <p><a id=\"lying\" href=\"{LYING}\">https://bank.com</a></p>\r\n"
    )
}

/// Newest first: the message carrying a calendar, a card and a forward; the letter with links; a
/// photo and a PDF.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    let store = with_account(dir);
    let now = chrono::Utc::now();
    let date = |hours: i64| (now - chrono::Duration::hours(hours)).to_rfc2822();
    let raws = [
        (
            "plans",
            carrying(
                "Plans",
                "plans",
                &date(1),
                &[
                    ("text/calendar", "invite.ics", ICS),
                    ("text/vcard", "ada.vcf", VCF),
                    ("message/rfc822", "Lunch on Friday.eml", EML),
                ],
            ),
        ),
        ("links", letter(&date(2))),
        (
            "holiday",
            carrying(
                "Holiday",
                "holiday",
                &date(3),
                &[
                    ("image/png", "beach.png", &png(400, 200)),
                    ("application/pdf", "report.pdf", TWO_PAGES),
                ],
            ),
        ),
    ];
    absorb(
        &store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        raws.into_iter()
            .map(|(uidl, raw)| Arrival {
                remote: RemoteRef::Pop {
                    uidl: uidl.to_owned(),
                },
                raw: raw.into_bytes(),
            })
            .collect(),
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

/// Fetches nothing: these messages have no remote parts.
struct NoFetch;
impl Fetch for NoFetch {
    fn get(&self, _: String, _: Box<dyn FnOnce(Vec<u8>) + Send>) {}
}

/// Opens nothing, and keeps what it was asked to open.
#[derive(Clone, Default)]
struct Opened(Arc<Mutex<Vec<String>>>);
impl Browse for Opened {
    fn open(&self, url: &str) {
        self.0.lock().unwrap().push(url.to_owned());
    }
}

/// The window as `launch` wires it, over the seeded store, a link opened from it going to `browse`.
fn open(browse: Opened) -> (Harness, tempfile::TempDir) {
    open_in(browse, false)
}

/// [`open`], in the dark scheme when `dark`.
fn open_in(browse: Opened, dark: bool) -> (Harness, tempfile::TempDir) {
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
    let config = if dark {
        let mut env = ds_settings::Environment::default();
        env.system.scheme = Scheme::Dark;
        config.with_context(env)
    } else {
        config
    };
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

/// The title of what Peek shows, once it is drawn.
const OWN_TITLE: &str = ".ds-peek .peek-own-title";

fn titled(harness: &mut Harness, title: &str) {
    until(harness, title, |h| {
        h.text_of(OWN_TITLE).as_deref() == Some(title)
    });
}

/// A calendar file, a contact card and a forwarded message each open in Peek as mailo reads them:
/// the event, the person, the message's head and plain text. The right arrow moves to the next
/// attachment and stops at the last; the left moves back; Esc closes and leaves the conversation.
#[test]
fn a_calendar_a_card_and_a_forward_open_in_peek_and_the_arrows_move_between_them() {
    let (mut harness, _dir) = open(Opened::default());
    let harness = &mut harness;
    open_row(harness, 1);
    until(harness, "the strip", |h| {
        h.count(".attachments .ds-list-item") == 3
    });
    let tile = format!("{} .att-mark", item(1));
    until(harness, "the calendar's tile", |h| h.count(&tile) == 1);
    let at = centre(harness, &tile);
    harness.click(at);
    titled(harness, "Design review");
    assert_eq!(
        harness.attr(PEEK, "aria-label").as_deref(),
        Some("invite.ics")
    );
    let facts = harness.text_of(".ds-peek .peek-own").unwrap_or_default();
    assert!(facts.contains("Room 4"), "the event's place: {facts}");

    harness.key(Key::Right);
    titled(harness, "Ada Lovelace");
    let facts = harness.text_of(".ds-peek .peek-own").unwrap_or_default();
    assert!(
        facts.contains("ada@example.test"),
        "the card's address: {facts}"
    );

    harness.key(Key::Right);
    titled(harness, "Lunch on Friday");
    let text = harness
        .text_of(".ds-peek .peek-own-text")
        .unwrap_or_default();
    assert_eq!(text, "See you at noon.", "the forward's text");
    let facts = harness.text_of(".ds-peek .peek-own").unwrap_or_default();
    assert!(
        facts.contains("Grace <grace@example.test>"),
        "the forward's sender: {facts}"
    );

    harness.key(Key::Right);
    harness.advance(ms(300));
    assert_eq!(
        harness.text_of(OWN_TITLE).as_deref(),
        Some("Lunch on Friday"),
        "the right arrow moved past the last attachment"
    );
    harness.key(Key::Left);
    titled(harness, "Ada Lovelace");

    harness.key(Key::Escape);
    until(harness, "Peek to close", |h| h.count(PEEK) == 0);
    assert_eq!(
        harness.count(".attachments .ds-list-item"),
        3,
        "Esc closed the conversation along with Peek"
    );
}

/// The tiles are on the keyboard's path: Tab reaches one, and Space opens it in Peek; Peek's
/// close tool closes it.
#[test]
fn space_on_a_focused_attachment_opens_it_in_peek() {
    let (mut harness, _dir) = open(Opened::default());
    let harness = &mut harness;
    open_row(harness, 1);
    let tile = format!("{} .att-mark", item(2));
    until(harness, "the card's tile", |h| h.count(&tile) == 1);
    for _ in 0..120 {
        if harness.is_focused(&tile) {
            break;
        }
        harness.key(Key::Tab);
        harness.advance(ms(20));
    }
    assert!(
        harness.is_focused(&tile),
        "Tab never reached the card's tile:\n{}",
        harness.html()
    );
    assert_eq!(harness.count(PEEK), 0, "Peek opened on Tab");
    harness.key(Key::Space);
    titled(harness, "Ada Lovelace");
    // The head shares the close tool's row; the tool still takes the press.
    let at = centre(harness, ".ds-peek-tools .ds-button");
    harness.click(at);
    until(harness, "Peek to close", |h| h.count(PEEK) == 0);
}

const MENU_ITEM: &str = ".ds-menu .ds-menu-item";
const PREVIEW_LINK: &str = ".ds-menu .ds-menu-item:nth-child(2)";

/// Right click the frame's link `selector` and pick Preview Link.
fn preview_link(harness: &mut Harness, selector: &str) {
    let link = harness
        .frame(FRAME)
        .and_then(|f| f.centre(selector))
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    harness.press(link, PointerButton::Secondary);
    until(harness, "the link's menu", |h| h.count(MENU_ITEM) == 3);
    assert_eq!(
        harness.text_of(PREVIEW_LINK).as_deref().map(str::trim),
        Some(copy::menus::PREVIEW_LINK),
        "the menu's second row"
    );
    let at = centre(harness, PREVIEW_LINK);
    harness.click(at);
    until(harness, "the link's card", |h| {
        h.count(".ds-peek .peek-link") == 1
    });
}

/// A link's card says what the link said, the site it goes to and the whole address; a link whose
/// text names another site says so. Esc closes the card without going anywhere; Open Link and
/// Primary+Return go there, through the window's opener, and close it.
#[test]
fn a_links_card_shows_where_it_goes_and_warns_when_its_text_lies() {
    let opened = Opened::default();
    let (mut harness, _dir) = open(opened.clone());
    let harness = &mut harness;
    open_row(harness, 2);
    until(harness, "the letter's frame", |h| {
        h.frame(FRAME)
            .is_some_and(|f| f.text().contains("the agenda"))
    });

    preview_link(harness, "#honest");
    assert_eq!(
        harness.attr(PEEK, "aria-label").as_deref(),
        Some(copy::aria::LINK_DESTINATION)
    );
    assert_eq!(
        harness.text_of(".peek-link-title").as_deref(),
        Some("the agenda")
    );
    assert_eq!(
        harness.text_of(".peek-link-site").as_deref(),
        Some("example.test")
    );
    assert_eq!(harness.text_of(".peek-link-url").as_deref(), Some(HONEST));
    assert_eq!(
        harness.count(".peek-link .ds-inline-banner"),
        0,
        "an honest link warned"
    );
    harness.key(Key::Escape);
    until(harness, "Peek to close", |h| h.count(PEEK) == 0);
    assert!(opened.0.lock().unwrap().is_empty(), "Esc opened the link");

    preview_link(harness, "#honest");
    harness.chord(&[PRIMARY], Key::Enter);
    until(harness, "Peek to close", |h| h.count(PEEK) == 0);
    assert_eq!(*opened.0.lock().unwrap(), vec![HONEST.to_owned()]);

    preview_link(harness, "#lying");
    assert_eq!(
        harness.text_of(".peek-link-site").as_deref(),
        Some("attacker.test")
    );
    let warning = harness
        .text_of(".peek-link .ds-inline-banner")
        .unwrap_or_default();
    assert!(
        warning.contains(&copy::errors::link_goes_elsewhere(
            "attacker.test",
            "bank.com"
        )),
        "the lying link's warning: {warning:?}"
    );
    let open_link = (1..=2)
        .map(|n| format!(".peek-link-actions .ds-button:nth-of-type({n})"))
        .find(|b| harness.text_of(b).as_deref() == Some(copy::controls::OPEN_LINK))
        .expect("Open Link");
    let at = centre(harness, &open_link);
    harness.click(at);
    until(harness, "Peek to close", |h| h.count(PEEK) == 0);
    assert_eq!(
        *opened.0.lock().unwrap(),
        vec![HONEST.to_owned(), LYING.to_owned()]
    );
}

/// Picture generator (ignored): Peek on a photo, on a PDF, on a forwarded message and on a link's
/// card, honest and lying, in light and dark. Set MAILO_SHOTS to a directory.
#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn peek_screens() {
    let out = std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS names the directory");
    let shot = |harness: &mut Harness, name: &str, dark: bool| {
        harness.advance(ms(600));
        let file = format!("{out}/{name}-{}.png", if dark { "dark" } else { "light" });
        harness.render().unwrap().save(file).unwrap();
    };
    for dark in [false, true] {
        let (mut harness, _dir) = open_in(Opened::default(), dark);
        let harness = &mut harness;
        open_row(harness, 3);
        let photo = format!("{} .att-thumb", item(1));
        until(harness, "the photo's tile", |h| {
            h.count(&format!("{photo} img")) == 1
        });
        let at = centre(harness, &photo);
        harness.click(at);
        until(harness, "the photo in Peek", |h| {
            h.count(".ds-peek .peek-picture") == 1
        });
        shot(harness, "peek-image", dark);
        harness.key(Key::Right);
        until(harness, "the PDF in Peek", |h| {
            h.count(".ds-peek .peek-page") == 1
        });
        shot(harness, "peek-pdf", dark);
        harness.key(Key::Escape);

        open_row(harness, 1);
        let forward = format!("{} .att-mark", item(3));
        until(harness, "the forward's tile", |h| h.count(&forward) == 1);
        let at = centre(harness, &forward);
        harness.click(at);
        titled(harness, "Lunch on Friday");
        shot(harness, "peek-message", dark);
        harness.key(Key::Escape);

        open_row(harness, 2);
        until(harness, "the letter's frame", |h| {
            h.frame(FRAME)
                .is_some_and(|f| f.text().contains("the agenda"))
        });
        preview_link(harness, "#honest");
        shot(harness, "peek-link", dark);
        harness.key(Key::Escape);
        until(harness, "Peek to close", |h| h.count(PEEK) == 0);
        preview_link(harness, "#lying");
        shot(harness, "peek-link-lying", dark);
    }
}
