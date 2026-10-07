//! Lane 5: links in a message. Hovering one shows where it really goes; a web link opens in the
//! browser without its tracking parameters, a mailto link goes where a mailto link goes, and a
//! `javascript:` link does nothing.

use ds::prelude::Point;
use ds_harness::{Driver, Query};

use super::drive::Drive;
use super::window::{Window, deliver, hours_ago, ms};

const SUBJECT: &str = "Three links";
const FRAME: &str = "article.frame iframe.html";

fn linked(store: &mail_store::SqliteStore) {
    let raw = format!(
        "From: Grace Hopper <grace@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {SUBJECT}\r\nDate: {}\r\nMessage-ID: <links1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/html; charset=UTF-8\r\n\r\n\
         <p>Read <a href=\"https://news.example.test/story?id=7&amp;utm_source=mail&amp;utm_campaign=fall\">the story</a>,\
          write to <a href=\"mailto:grace@example.test?subject=Hello\">Grace</a>,\
          or <a href=\"javascript:alert(1)\">run this</a>.</p>\r\n",
        hours_ago(0)
    );
    deliver(store, "links1", &raw);
}

/// Where the `n`th link in the message's frame is drawn, in the window.
fn link(window: &Window, n: usize) -> Point {
    let selector = ["a", "a + a", "a + a + a"][n - 1];
    window
        .harness
        .frame(FRAME)
        .and_then(|frame| frame.centre(selector))
        .unwrap_or_else(|| panic!("link {n} is not drawn"))
}

#[test]
fn a_hovered_link_names_its_destination_and_a_click_opens_only_what_is_safe() {
    let mut window = Window::open(linked);
    window.open_subject(SUBJECT);
    window.until("the message's links are drawn", |h| {
        h.frame(FRAME)
            .is_some_and(|frame| frame.centre("a + a + a").is_some())
    });

    // Hover: the pill names the real destination, the tracking parameters gone.
    let story = link(&window, 1);
    window.harness.pointer_move(story);
    window.until("the link pill shows", |h| h.count(".ds-link-pill") == 1);
    let pill = window.text(".ds-link-pill");
    assert!(
        pill.contains("example.test") && !pill.contains("utm_"),
        "the pill does not name the destination: {pill:?}"
    );
    assert_eq!(
        window
            .harness
            .attr(".ds-link-pill", "data-truth")
            .as_deref(),
        Some("honest"),
        "a link whose text names no other place is not honest"
    );

    // Click: the browser is asked for the cleaned address.
    window.harness.click(story);
    window.harness.advance(ms(300));
    assert_eq!(window.opened(), ["https://news.example.test/story?id=7"]);

    // mailto: handed to whatever opens links, as the desktop's mail handler would get it.
    let mail = link(&window, 2);
    window.harness.click(mail);
    window.harness.advance(ms(300));
    assert_eq!(
        window.opened(),
        [
            "https://news.example.test/story?id=7",
            "mailto:grace@example.test?subject=Hello"
        ]
    );
    assert_eq!(
        window.harness.count(".cpage"),
        0,
        "a mailto link opened a composer"
    );

    // javascript: nothing at all.
    let script = link(&window, 3);
    window.harness.pointer_move(script);
    window.harness.click(script);
    window.harness.advance(ms(300));
    assert_eq!(window.opened().len(), 2, "javascript: opened something");
    assert!(
        window
            .harness
            .frame(FRAME)
            .is_some_and(|frame| frame.text().contains("run this")),
        "the frame navigated away"
    );
}
