//! The reader's message body scrolls on Blitz (`native`), in the real window: a body longer
//! than the pane moves under a mouse wheel turned over it, as the window delivers it (through
//! quire's scroll engine), for a plain-text body, an HTML body and the last message of a thread.
//! A touchpad is not driven here: the harness's `Input::Fingers` drops a wheel the engine passes
//! on, where the window hands it to the document (gap(quire)).
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use dioxus::prelude::Modifiers;
use ds::prelude::*;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Input, Query, Viewport};
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

use crate::drive;
use drive::Drive;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c3"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Eighty numbered lines: far taller than an 800 px window.
fn lines() -> Vec<String> {
    (1..=80)
        .map(|n| format!("Line {n} of a long letter."))
        .collect()
}

fn plain(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: A long plain letter\r\n\
         Date: {date}\r\nMessage-ID: <plain@example.test>\r\n\r\n{}\r\n",
        lines().join("\r\n")
    )
}

fn html(date: &str) -> String {
    let paragraphs: String = lines().iter().map(|l| format!("<p>{l}</p>")).collect();
    format!(
        "From: news@example.test\r\nTo: me@example.test\r\nSubject: A long HTML letter\r\n\
         Date: {date}\r\nMessage-ID: <html@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n<html><body>{paragraphs}</body></html>\r\n"
    )
}

/// The first of a thread of two long letters.
fn first(date: &str) -> String {
    format!(
        "From: grace@example.test\r\nTo: me@example.test\r\nSubject: A long thread\r\n\
         Date: {date}\r\nMessage-ID: <t1@example.test>\r\n\r\n{}\r\n",
        lines().join("\r\n")
    )
}

/// The reply that makes it a thread.
fn reply(date: &str) -> String {
    format!(
        "From: grace@example.test\r\nTo: me@example.test\r\nSubject: Re: A long thread\r\n\
         Date: {date}\r\nMessage-ID: <t2@example.test>\r\nIn-Reply-To: <t1@example.test>\r\n\
         References: <t1@example.test>\r\n\r\n{}\r\n",
        lines().join("\r\n")
    )
}

/// A letter's raw bytes, given its date.
type Letter = fn(&str) -> String;

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
    // Listed newest first: the plain letter, the HTML letter, then the thread.
    let bodies: [(i64, Letter); 4] = [(1, plain), (2, html), (4, first), (3, reply)];
    for (n, (hours, body)) in bodies.iter().enumerate() {
        let date = (now - chrono::Duration::hours(*hours)).to_rfc2822();
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
                    uidl: format!("scroll{n}"),
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

fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let contexts = mail_app::ui::native::contexts(
        seeded(dir.path()),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Wall)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    (harness, dir)
}

fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread")
}

/// Open row `n` and wait until its last frame has a document with `marker` in it.
fn open_row(harness: &mut Harness, n: usize, marker: &str) {
    let subject = format!("{} .ds-thread-sub", row(n));
    for _ in 0..50 {
        if harness.rect(&subject).is_some() {
            break;
        }
        harness.advance(ms(100));
    }
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    for _ in 0..100 {
        harness.advance(ms(100));
        if harness
            .frame(FRAME)
            .is_some_and(|frame| frame.centre(marker).is_some())
        {
            return;
        }
    }
    panic!("row {n}'s frame never showed {marker}:\n{}", harness.html());
}

/// The last message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// Where `marker` in the last frame is drawn on screen, and where the reader's body is.
fn place(harness: &Harness, marker: &str) -> (f32, f32) {
    let inner = harness
        .frame(FRAME)
        .and_then(|frame| frame.centre(marker))
        .map_or(f32::NAN, |p| p.y.0);
    let outer = harness
        .rect(".reader-body article.frame:last-of-type")
        .map_or(f32::NAN, |r| r.origin.y.0);
    (inner, outer)
}

/// A point over the last frame, in the window's coordinates.
fn over_body(harness: &Harness) -> Point {
    let rect = harness
        .rect(FRAME)
        .unwrap_or_else(|| panic!("{FRAME} is not drawn:\n{}", harness.html()));
    let bottom = (rect.origin.y.0 + rect.size.height.0).min(VIEW.height as f32 - 20.0);
    Point {
        x: Px(rect.origin.x.0 + rect.size.width.0 / 2.0),
        y: Px((rect.origin.y.0 + bottom) / 2.0),
    }
}

/// Turn a mouse wheel three clicks down, six times, over the body, as the window delivers it,
/// and say whether anything in the body moved.
fn scrolls(harness: &mut Harness, marker: &str) -> bool {
    let at = over_body(harness);
    let before = place(harness, marker);
    for _ in 0..6 {
        harness.send(Input::Detents {
            at,
            x: 0.0,
            y: -3.0,
            held: Modifiers::empty(),
        });
        harness.advance(ms(16));
    }
    harness.advance(ms(800));
    let after = place(harness, marker);
    eprintln!("wheel at {at:?}: (frame marker y, article y) {before:?} -> {after:?}");
    after.0 < before.0 - 1.0 || after.1 < before.1 - 1.0
}

/// Each long letter scrolls under the wheel: a plain one, an HTML one, and a thread. Each row
/// opens in a window of its own, so no row starts where another's wheel left the reader.
#[test]
fn each_long_letter_scrolls_under_the_wheel() {
    // (what, row, a marker in its last frame)
    const CASES: [(&str, usize, &str); 3] = [
        ("a long plain letter", 1, "body"),
        ("a long HTML letter", 2, "p"),
        ("a long thread", 3, "body"),
    ];
    for (what, n, marker) in CASES {
        let (mut harness, _dir) = open();
        open_row(&mut harness, n, marker);
        let frame = harness.rect(FRAME);
        let body = harness.rect(".reader-body");
        assert!(
            scrolls(&mut harness, marker),
            "{what} (row {n}): the body did not move under the wheel (frame {frame:?}, reader-body {body:?})"
        );
    }
}
