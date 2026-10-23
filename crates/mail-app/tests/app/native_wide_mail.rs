//! A message laid out wider than the reader is drawn smaller to fit it, as Mail draws it, on
//! Blitz (`native`) in the real window: a newsletter whose tables and pictures are fixed at
//! 800 px, in a pane narrower than that, is shrunk until its widest table is no wider than the
//! frame, and a message that fits is left at its own size.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use ds::prelude::*;
use ds_harness::{Clock, Harness};
use mail_core::SqliteStore;
use std::sync::Arc;

use crate::drive;
use crate::native::ms;
use crate::native::thread;
use crate::opening::{Open, Wait, open_window};
use crate::store::{self, acct_account};
use drive::Drive;

/// A newsletter as a bulk sender lays one out: centred, a fixed 800 px table holding a fixed
/// 800 px table, as `table-layout: fixed` keeps it, that no `max-width` narrows.
fn wide(date: &str) -> String {
    format!(
        "From: news@example.test\r\nTo: me@example.test\r\nSubject: A wide newsletter\r\n\
         Date: {date}\r\nMessage-ID: <wide@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n<html><body><div align=\"center\">\
         <table border=\"0\" width=\"800\"><tr><td><table id=\"inner\" width=\"800px\" \
         style=\"table-layout: fixed\"><tr><td>Wide words</td><td>at the right edge</td></tr>\
         </table></td></tr></table></div></body></html>\r\n"
    )
}

/// An ordinary HTML letter, which fits any pane.
fn narrow(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: A short letter\r\n\
         Date: {date}\r\nMessage-ID: <narrow@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n<html><body><p id=\"words\">Short words\
         </p></body></html>\r\n"
    )
}

/// A letter's raw bytes, given its date.
type Letter = fn(&str) -> String;

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    let store = store::account(dir, acct_account(), "me@example.test", None);
    let now = chrono::Utc::now();
    // Listed newest first: the newsletter, then the letter.
    let bodies: [(i64, Letter); 2] = [(1, wide), (2, narrow)];
    for (n, (hours, body)) in bodies.iter().enumerate() {
        let date = (now - chrono::Duration::hours(*hours)).to_rfc2822();
        store::deliver(&store, acct_account(), &format!("wide{n}"), body(&date));
    }
    Arc::new(store)
}

/// The message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// Open row `n` and wait until its frame shows `marker`.
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
            .is_some_and(|frame| frame.count(marker) == 1)
        {
            return;
        }
    }
    panic!("row {n}'s frame never showed {marker}:\n{}", harness.html());
}

/// How wide `selector` is drawn in the message's frame, once it has stopped changing.
fn drawn(harness: &mut Harness, selector: &str) -> f32 {
    let read = |h: &Harness| {
        h.frame(FRAME)
            .and_then(|frame| frame.width(selector))
            .map(|px| px.0)
    };
    let (mut was, mut same) = (read(harness), 0);
    for _ in 0..100 {
        harness.advance(ms(150));
        let now = read(harness);
        same = if now == was { same + 1 } else { 0 };
        if same == 4 {
            return now.unwrap_or_else(|| panic!("{selector} is not drawn"));
        }
        was = now;
    }
    panic!("{selector} never settled:\n{}", harness.html());
}

#[test]
fn a_message_wider_than_the_reader_is_drawn_smaller_to_fit_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = open_window(
        &seeded(dir.path()),
        Open {
            clock: Clock::Wall,
            wait: Wait::Entrances,
            ..Open::default()
        },
    );
    shown(&mut harness, 1, "#inner");
    let frame = harness
        .rect(FRAME)
        .expect("the frame is laid out")
        .size
        .width
        .0;
    assert!(
        frame < 800.0,
        "the pane is wider than the newsletter: {frame}"
    );
    let table = drawn(&mut harness, "#inner");
    assert!(
        table <= frame && table > frame * 0.8,
        "the 800 px table is drawn {table} px wide in a {frame} px frame"
    );

    shown(&mut harness, 2, "#words");
    let words = drawn(&mut harness, "#words");
    let frame = harness
        .rect(FRAME)
        .expect("the frame is laid out")
        .size
        .width
        .0;
    // The body's 16 px inset either side, at full size.
    assert!(
        (words - (frame - 32.0)).abs() < 1.0,
        "the letter that fits is drawn {words} px wide in a {frame} px frame"
    );
}
