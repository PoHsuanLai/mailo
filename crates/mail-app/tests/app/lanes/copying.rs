//! Copying from a message. A message's body is a frame with a document of its own, so what is
//! selected in it is that document's, not the window's: words dragged over in a sender's HTML or
//! a plain-text message are copied by ⌘C, a right click on them offers Copy, and ⌘A with the
//! keyboard in the message selects the message's words, not every conversation in the list.

use ds::base::press::PointerButton;
use ds::prelude::{Point, Px};
use ds_harness::Harness;
use mail_ui::ui::copy;

use super::drive::{Drive, Key, PRIMARY};
use super::look::row;
use super::window::{Window, deliver, ms};

/// A security notice laid out as a bulk sender's tool lays one out: `multipart/alternative`, its
/// HTML a `<body style="margin:0;padding:0">` holding flush paragraphs and no centring table.
const NOTICE: &str = include_str!("../../../../mail-core/tests/fixtures/flush-notice.eml");
const NOTICE_SUBJECT: &str = "Please rotate a key you used";
const PLAIN_SUBJECT: &str = "Plain words";
/// The open message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";
const MENU: &str = ".ds-menu .ds-menu-item";

fn letters(store: &mail_core::SqliteStore) {
    deliver(store, "notice", NOTICE);
    let plain = format!(
        "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: {PLAIN_SUBJECT}\r\n\
         Date: {}\r\nMessage-ID: <plain-copy@example.test>\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nFirst line to copy.\r\n\r\nSecond line.\r\n",
        chrono::Utc::now().to_rfc2822()
    );
    deliver(store, "plain", &plain);
}

/// A point on the first line of the frame's first element matching `selector`, `dx` from the
/// element's left edge, in the window's coordinates.
fn on_line(harness: &Harness, selector: &str, dx: f32) -> Option<Point> {
    let frame = harness.rect(FRAME)?;
    harness.with_doc(|doc| {
        // A selector of this file's that does not parse finds nothing, as a missing one does.
        let iframe = doc.query_selector(FRAME).ok()??;
        let sub = doc.get_node(iframe)?.subdoc()?.inner();
        // The same for the one asked about.
        let found = sub.query_selector(selector).ok()??;
        let rect = sub.get_client_bounding_rect(found)?;
        Some(Point {
            x: Px(frame.origin.x.0 + rect.x as f32 + dx),
            y: Px(frame.origin.y.0 + (rect.y + (rect.height / 2.0).min(10.0)) as f32),
        })
    })
}

/// What the open message's frame has selected.
fn selected(harness: &Harness) -> Option<String> {
    harness.with_doc(|doc| {
        // A selector that does not parse finds no frame, as a missing frame does.
        let iframe = doc.query_selector(FRAME).ok()??;
        doc.get_node(iframe)?.subdoc()?.inner().get_selected_text()
    })
}

/// Open `subject` and wait for its frame's `selector` to be laid out where it is drawn: the
/// pointer moved over the window once, as a person's is, so the frame's layout is the settled one.
fn open(window: &mut Window, subject: &str, selector: &str) {
    window.open_subject(subject);
    let what = format!("{subject:?}'s words are drawn");
    window.until(&what, |h| on_line(h, selector, 0.0).is_some());
    let frame = window.harness.rect(FRAME).expect("the frame");
    window.harness.pointer_move(Point {
        x: Px(frame.origin.x.0 + frame.size.width.0 / 2.0),
        y: Px(frame.origin.y.0 + frame.size.height.0 - 8.0),
    });
    window.harness.advance(ms(300));
}

/// Drag over `selector`'s first line in the open message, from `from` to `to` px into it, and
/// return what the frame then holds selected.
fn drag_over(window: &mut Window, selector: &str, from: f32, to: f32) -> Option<String> {
    let start = on_line(&window.harness, selector, from).expect("drawn");
    let end = on_line(&window.harness, selector, to).expect("drawn");
    window.harness.drag(start, end, 8);
    window.harness.advance(ms(200));
    selected(&window.harness)
}

/// How many rows of the list are picked.
fn picked(harness: &Harness) -> usize {
    (1..=harness.count(".list .ds-thread"))
        .filter(|n| {
            harness
                .attr(&format!("{} .ds-row", row(*n)), "aria-selected")
                .as_deref()
                == Some("true")
        })
        .count()
}

#[test]
fn words_dragged_over_in_a_message_are_copied_by_command_c() {
    let mut window = Window::open(letters);

    // A sender's HTML, its paragraphs flush with the frame.
    open(&mut window, NOTICE_SUBJECT, "p + p");
    let words = drag_over(&mut window, "p + p", 20.0, 200.0).expect("the drag selects words");
    assert!(words.contains("recently"), "the drag selected {words:?}");
    window.press(&[PRIMARY], Key::Char('c'), 1);
    assert_eq!(window.harness.clipboard_text(), Some(words));

    // A plain-text message, the same way.
    open(&mut window, PLAIN_SUBJECT, "body");
    // The words sit 18 px in, inside the frame's gutter; a drag begins on them, not in it.
    let words = drag_over(&mut window, "body", 19.0, 138.0).expect("the drag selects words");
    assert!(words.starts_with("First"), "the drag selected {words:?}");
    window.press(&[PRIMARY], Key::Char('c'), 1);
    assert_eq!(window.harness.clipboard_text(), Some(words));
}

#[test]
fn a_right_click_on_selected_words_in_a_message_offers_copy() {
    let mut window = Window::open(letters);
    open(&mut window, NOTICE_SUBJECT, "p + p");
    let words = drag_over(&mut window, "p + p", 20.0, 200.0).expect("the drag selects words");

    let on = on_line(&window.harness, "p + p", 60.0).expect("drawn");
    window.harness.press(on, PointerButton::Secondary);
    window.until("the selection's menu", |h| h.count(MENU) == 1);
    window.menu_item(copy::menus::COPY);
    window.until("the menu closes", |h| h.count(MENU) == 0);
    assert_eq!(window.harness.clipboard_text(), Some(words.clone()));
    assert_eq!(
        selected(&window.harness),
        Some(words),
        "the right click let the selection go"
    );

    // With nothing selected, a right click on the words offers nothing, as before.
    let off = on_line(&window.harness, "p", 20.0).expect("drawn");
    window.harness.click(off);
    window.harness.advance(ms(300));
    assert_eq!(
        selected(&window.harness),
        None,
        "a click kept the selection"
    );
    window.harness.press(off, PointerButton::Secondary);
    window.harness.advance(ms(300));
    assert_eq!(window.harness.count(MENU), 0, "a menu with nothing to copy");
}

#[test]
fn command_a_in_a_message_selects_its_words_and_not_the_list() {
    let mut window = Window::open(letters);

    for (subject, selector, first, last) in [
        (NOTICE_SUBJECT, "p", "Hi there,", "The Example Team"),
        (PLAIN_SUBJECT, "body", "First line to copy.", "Second line."),
    ] {
        open(&mut window, subject, selector);
        // A click on the words gives the message the keyboard.
        let on = on_line(&window.harness, selector, 30.0).expect("drawn");
        window.harness.click(on);
        window.harness.advance(ms(300));
        let before = picked(&window.harness);

        window.press(&[PRIMARY], Key::Char('a'), 1);
        // Nothing selected reads as no words, which the assert below names.
        let all = selected(&window.harness).unwrap_or_default();
        assert!(
            all.contains(first) && all.contains(last),
            "⌘A in {subject:?} selected {all:?}"
        );
        assert_eq!(
            picked(&window.harness),
            before,
            "⌘A in a message picked conversations in the list"
        );
        window.press(&[PRIMARY], Key::Char('c'), 1);
        assert_eq!(window.harness.clipboard_text(), Some(all));
    }
}
