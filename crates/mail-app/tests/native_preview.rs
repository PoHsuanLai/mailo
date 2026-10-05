//! Attachment previews in the real window: a stored picture's and a stored PDF's thumbnails in
//! the reader's strip, the viewer they open (Esc closes it, the arrows turn a PDF's pages, Save
//! stays), no preview and no request for a part still on the server, and a note instead of a
//! decode for a picture that claims too many pixels.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`, and every
//! request its documents make beyond inline `data:` goes to a recorder that refuses it.

use base64::Engine as _;
use ds::prelude::ShortcutKey as Key;
use ds_blitz::{AppNet, NetDecision, NetPolicy, NetReply, NetRequest};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use mail_domain::*;
use mail_runtime::assemble::absorb_rebuilt_into;
use mail_runtime::{Arrival, Destination, absorb};
use mail_store::{SqliteStore, Store};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c7"));

const VIEW: Viewport = Viewport {
    width: 1280,
    height: 900,
    scale_percent: 100,
};

/// Two pages: 200 × 100 points, then 200 × 300.
const TWO_PAGES: &[u8] = include_bytes!("../../mail-core/tests/fixtures/two-pages.pdf");

/// Every request a document made that was not inline, refused.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl AppNet for Recorder {
    fn decide(&self, request: &NetRequest) -> NetDecision {
        self.0.lock().unwrap().push(request.url().to_owned());
        NetDecision::Deny
    }

    fn fetch(&self, _request: NetRequest, _reply: NetReply) {}
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 200]));
    let mut out = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A PNG whose header claims 100 000 × 100 000 pixels: 30 GB decoded, a few dozen bytes sent.
fn bomb() -> Vec<u8> {
    let mut png = png(1, 1);
    png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A message carrying `parts`: `(declared type, name, bytes)`.
fn carrying(subject: &str, id: &str, date: &str, parts: &[(&str, &str, &[u8])]) -> String {
    let mut raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\nDate: {date}\r\n\
         Message-ID: <{id}@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nAttached.\r\n"
    );
    for (mime, name, bytes) in parts {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let lines: Vec<&str> = encoded
            .as_bytes()
            .chunks(76)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect();
        raw.push_str(&format!(
            "--mix\r\nContent-Type: {mime}; name=\"{name}\"\r\n\
             Content-Disposition: attachment; filename=\"{name}\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{}\r\n",
            lines.join("\r\n")
        ));
    }
    raw.push_str("--mix--\r\n");
    raw
}

/// A large IMAP message as the store holds it: its photo left on the server (`reconstruct`).
fn rebuilt(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: The big photo\r\nDate: {date}\r\n\
         Message-ID: <big@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nToo big to send whole.\r\n\
         --mix\r\nContent-Type: image/png; name=\"big.png\"\r\n\
         Content-Disposition: attachment; filename=\"big.png\"\r\n\
         Content-Transfer-Encoding: base64\r\nX-Mailo-Remote-Section: 2\r\n\
         X-Mailo-Remote-Octets: 9000000\r\n\r\n\r\n--mix--\r\n"
    )
}

/// Newest first: the photo and the report; the bomb and an SVG; the photo left on the server.
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
    }
    let now = chrono::Utc::now();
    let date = |hours: i64| (now - chrono::Duration::hours(hours)).to_rfc2822();
    let inbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let arrival = |uidl: &str, raw: String| Arrival {
        remote: RemoteRef::Pop {
            uidl: uidl.to_owned(),
        },
        raw: raw.into_bytes(),
    };
    let photo = png(400, 200);
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"https://tracker.example.test/p.png\"/></svg>";
    absorb(
        &store,
        ACCOUNT,
        inbox.clone(),
        Some(SyncCursor::Pop),
        vec![
            arrival(
                "photo",
                carrying(
                    "Holiday",
                    "holiday",
                    &date(1),
                    &[
                        ("image/png", "beach.png", &photo),
                        ("application/pdf", "report.pdf", TWO_PAGES),
                    ],
                ),
            ),
            arrival(
                "bomb",
                carrying(
                    "Scans",
                    "scans",
                    &date(2),
                    &[
                        ("image/png", "huge.png", &bomb()),
                        ("image/svg+xml", "logo.svg", svg),
                    ],
                ),
            ),
        ],
        false,
        now,
    )
    .unwrap();
    absorb_rebuilt_into(
        &store,
        ACCOUNT,
        Destination {
            mailbox: inbox,
            role: MailboxRole::Inbox,
        },
        vec![arrival("big", rebuilt(&date(3)))],
        now,
    )
    .unwrap();
    Arc::new(store)
}

struct Window {
    harness: Harness,
    store: Arc<SqliteStore>,
    asked: Arc<Recorder>,
    _dir: tempfile::TempDir,
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn open() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let contexts = mail_app::ui::native::contexts(
        store.clone(),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    );
    let asked = Arc::new(Recorder::default());
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Custom(asked.clone()))
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    Window {
        harness,
        store,
        asked,
        _dir: dir,
    }
}

fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-row .ds-thread-sub");
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(ds::prelude::Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

fn click(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(100));
}

/// The strip's `n`th row (1-based). The strip is quire's list.
fn item(n: usize) -> String {
    format!(".attachments .ds-list > .ds-list-item:nth-child({n})")
}

const VIEWER: &str = ".viewer-wrap .viewer";
const PICTURE: &str = ".viewer .viewer-picture";

/// The viewer's head button that says `label`.
fn head_button(harness: &Harness, label: &str) -> String {
    (1..=6)
        .map(|n| format!(".viewer-head .ds-button:nth-of-type({n})"))
        .find(|selector| harness.text_of(selector).as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} in the viewer's head:\n{}", harness.html()))
}

/// A stored PNG leads its row with a thumbnail drawn from its bytes, and a press opens the viewer
/// on it, larger, with Save; Esc closes the viewer and leaves the conversation open.
#[test]
fn a_stored_picture_shows_a_thumbnail_that_opens_the_viewer() {
    let mut window = open();
    let harness = &mut window.harness;
    open_row(harness, 1);
    let thumb = format!("{} .att-thumb img", item(1));
    settle_until(harness, |h| h.count(&thumb) == 1);
    let src = harness.attr(&thumb, "src").unwrap();
    assert!(src.starts_with("data:image/png;base64,"), "{src}");
    let rect = harness.rect(&thumb).unwrap();
    // 400 × 200 fitted into 56 × 56 at its own aspect.
    assert!(
        (rect.size.width.0 - 56.0).abs() < 1.0 && (rect.size.height.0 - 28.0).abs() < 1.0,
        "the thumbnail is drawn at {:?}",
        rect.size
    );
    assert_eq!(harness.count(VIEWER), 0, "the viewer was open unasked");

    click(harness, &format!("{} .att-thumb", item(1)));
    settle_until(harness, |h| h.count(PICTURE) == 1);
    assert_eq!(
        harness.attr(".viewer", "aria-label").as_deref(),
        Some("beach.png")
    );
    let big = harness.attr(PICTURE, "src").unwrap();
    assert!(big.starts_with("data:image/png;base64,"));
    assert_ne!(big, src, "the viewer drew the thumbnail, not the picture");
    let drawn = harness.rect(PICTURE).unwrap();
    assert!(
        drawn.size.width.0 > 56.0,
        "the viewer's picture is no larger than the thumbnail: {:?}",
        drawn.size
    );
    head_button(harness, "Save");
    assert_eq!(
        harness.count(".viewer-page"),
        0,
        "a picture was given pages"
    );

    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(VIEWER) == 0);
    assert_eq!(
        harness.count(&thumb),
        1,
        "Esc closed the conversation along with the viewer"
    );
    assert!(
        window.asked.0.lock().unwrap().is_empty(),
        "the window asked the network for {:?}",
        window.asked.0.lock().unwrap()
    );
}

/// A stored PDF leads its row with quire's first-page thumbnail; the viewer shows its pages one
/// at a time, the arrows turn them, and they stop at the last.
#[test]
fn a_stored_pdf_shows_its_first_page_and_the_viewer_turns_its_pages() {
    let mut window = open();
    let harness = &mut window.harness;
    open_row(harness, 1);
    let thumb = format!("{} .att-thumb .ds-pdf-thumb", item(2));
    settle_until(harness, |h| {
        h.attr(&thumb, "data-state").as_deref() == Some("ready")
    });
    assert_eq!(
        harness.attr(&thumb, "aria-label").as_deref(),
        Some("report.pdf")
    );

    click(harness, &format!("{} .att-thumb", item(2)));
    settle_until(harness, |h| {
        h.text_of(".viewer-page").as_deref() == Some("Page 1 of 2")
    });
    let first = harness.attr(PICTURE, "src").unwrap();
    // The first page is 2:1, the second 2:3.
    let shape = |h: &Harness| {
        let r = h.rect(PICTURE).unwrap();
        r.size.width.0 / r.size.height.0
    };
    assert!((shape(harness) - 2.0).abs() < 0.05, "{}", shape(harness));

    harness.key(Key::Right);
    settle_until(harness, |h| {
        h.text_of(".viewer-page").as_deref() == Some("Page 2 of 2")
            && h.attr(PICTURE, "src").as_deref() != Some(first.as_str())
    });
    assert!(
        (shape(harness) - 2.0 / 3.0).abs() < 0.05,
        "{}",
        shape(harness)
    );
    let second = harness.attr(PICTURE, "src").unwrap();

    harness.key(Key::Right);
    harness.advance(ms(300));
    assert_eq!(
        harness.text_of(".viewer-page").as_deref(),
        Some("Page 2 of 2"),
        "the arrow turned past the last page"
    );
    assert_eq!(harness.attr(PICTURE, "src").unwrap(), second);

    harness.key(Key::Left);
    settle_until(harness, |h| {
        h.text_of(".viewer-page").as_deref() == Some("Page 1 of 2")
            && h.attr(PICTURE, "src").as_deref() == Some(first.as_str())
    });
    let next = head_button(harness, "Next page");
    click(harness, &next);
    settle_until(harness, |h| {
        h.text_of(".viewer-page").as_deref() == Some("Page 2 of 2")
    });
    head_button(harness, "Save");

    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(VIEWER) == 0);
    assert!(window.asked.0.lock().unwrap().is_empty());
}

/// A picture still on the server shows no preview, and drawing its row fetched nothing: the part
/// is still on the server and the window asked the network for nothing.
#[test]
fn a_part_still_on_the_server_shows_no_preview_and_fetches_nothing() {
    let mut window = open();
    let remote = window.store.offline(ACCOUNT).unwrap().parts_remote;
    assert_eq!(remote, 1, "the fixture's photo is not on the server");
    let harness = &mut window.harness;
    open_row(harness, 3);
    settle_until(harness, |h| h.count(".attachments .ds-list-item") == 1);
    harness.advance(ms(600));
    assert_eq!(harness.count(".attachments .att-thumb"), 0);
    assert_eq!(harness.count(".attachments .att-note"), 0);
    assert!(
        harness.text_of(&item(1)).unwrap().contains("Download"),
        "{}",
        harness.text_of(&item(1)).unwrap()
    );
    assert_eq!(
        window.store.offline(ACCOUNT).unwrap().parts_remote,
        remote,
        "drawing the row fetched the part"
    );
    assert!(
        window.asked.0.lock().unwrap().is_empty(),
        "the window asked the network for {:?}",
        window.asked.0.lock().unwrap()
    );
}

/// A picture whose header claims 100 000 × 100 000 pixels is refused with a note and never
/// decoded; an SVG is a file, not a picture.
#[test]
fn an_oversized_picture_is_refused_with_a_note_and_an_svg_is_not_drawn() {
    let mut window = open();
    let harness = &mut window.harness;
    open_row(harness, 2);
    let note = format!("{} .att-note", item(1));
    settle_until(harness, |h| h.count(&note) == 1);
    assert_eq!(
        harness.text_of(&note).as_deref(),
        Some("Too large to preview (100000 × 100000)")
    );
    assert_eq!(harness.count(".attachments .att-thumb"), 0);
    harness.advance(ms(300));
    assert_eq!(
        harness.count(&format!("{} .att-thumb", item(2))),
        0,
        "the SVG was drawn"
    );
    assert_eq!(harness.count(&format!("{} .att-note", item(2))), 0);
    assert!(
        window.asked.0.lock().unwrap().is_empty(),
        "the window asked the network for {:?}",
        window.asked.0.lock().unwrap()
    );
}
