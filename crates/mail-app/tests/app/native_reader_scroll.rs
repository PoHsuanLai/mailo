//! The reader scrolls on Blitz (`native`), in the real window, as the window delivers a mouse
//! wheel (through quire's scroll engine): a quick run of wheel clicks that starts over an earlier
//! message moves the conversation, not that message's frame.
//! A touchpad is not driven here: the harness's `Input::Fingers` drops a wheel the engine passes
//! on, where the window hands it to the document (gap(quire)).
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use dioxus::prelude::Modifiers;
use ds::prelude::*;
use ds_harness::{Clock, Harness, Input, Viewport};
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;

use crate::drive;
use crate::native::ms;
use crate::native::thread;
use crate::opening::{Open, Wait, open_window};
use drive::Drive;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c3"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

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
    let harness = open_window(
        &seeded(dir.path()),
        Open {
            clock: Clock::Wall,
            wait: Wait::Entrances,
            ..Open::default()
        },
    );
    (harness, dir)
}

/// Open row `n`, wait until its last frame has a document with `marker` in it, and then until
/// the conversation has settled: an earlier message's frame grows to its content after it loads,
/// which moves the last one down, and a wheel turned while that happens can look like no scroll.
fn open_row(harness: &mut Harness, n: usize, marker: &str) {
    shown(harness, n, marker);
    settle(harness);
}

/// Where the last message starts, until it is in the same place for three reads in a row.
fn settle(harness: &mut Harness) {
    let last = |h: &Harness| {
        h.rect(".reader-body article.frame:last-of-type")
            .map(|r| r.origin.y.0)
    };
    let (mut was, mut same) = (last(harness), 0);
    for _ in 0..100 {
        harness.advance(ms(100));
        let now = last(harness);
        same = if now == was { same + 1 } else { 0 };
        if same == 3 {
            return;
        }
        was = now;
    }
    panic!("the conversation never settled:\n{}", harness.html());
}

/// Open row `n` and wait until its last frame has a document with `marker` in it.
fn shown(harness: &mut Harness, n: usize, marker: &str) {
    let subject = format!("{} .ds-thread-sub", thread(n));
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

#[test]
fn a_wheel_burst_over_an_earlier_message_moves_the_conversation() {
    let (mut harness, _dir) = open();
    open_row(&mut harness, 3, "body");
    harness.advance(ms(1500));
    let early = ".reader-body article.frame:first-of-type iframe.html";
    let frame = harness.rect(early).expect("the first message's frame");
    let at = Point {
        x: Px(frame.origin.x.0 + frame.size.width.0 / 2.0),
        y: Px((frame.origin.y.0 + 20.0).max(130.0)),
    };
    // Where the first message's text sits, and where the last message starts: the text keeps its
    // place in the frame when the conversation, not the frame, moves.
    let text = |h: &Harness| h.frame(early).and_then(|f| f.centre("body")).map(|p| p.y.0);
    let last = |h: &Harness| {
        h.rect(".reader-body article.frame:last-of-type")
            .map(|r| r.origin.y.0)
    };
    let (text_was, last_was) = (text(&harness).expect("text"), last(&harness).expect("last"));
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
    let (text_now, last_now) = (text(&harness).expect("text"), last(&harness).expect("last"));
    assert!(
        last_was - last_now > 500.0,
        "the conversation moved: {last_was} -> {last_now}"
    );
    assert!(
        ((text_was - text_now) - (last_was - last_now)).abs() < 1.0,
        "the frame kept its own place: text {text_was} -> {text_now}, last {last_was} -> {last_now}"
    );
}
