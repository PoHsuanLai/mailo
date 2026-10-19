//! The composer is set as the reader sets a message, on Blitz (`native`), in the real window: a
//! new message's subject sits where a message's title does, at its size (20) and weight; its
//! header lines start at the reader's left edge, as a message's header does, each label on that
//! edge and every value at one x; its body starts where the reader's body
//! does, at the reader's size; and every trailing control (the To row's Bcc, Send ▾) ends
//! on one right edge. A reply under the thread lines up the same way, and its bar of buttons is
//! drawn inside the window.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use ds::prelude::*;
use ds_harness::{Harness, Viewport};

use crate::drive;
use crate::native::ms;
use crate::one_message::seeded;
use crate::opening::{Open, Wait, open_window};
use drive::{Drive, Key};

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// How far apart two edges may be and still read as one.
const SLACK: f32 = 2.0;

fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let harness = open_window(
        &seeded(dir.path()),
        Open {
            wait: Wait::Entrances,
            ..Open::default()
        },
    );
    (harness, dir)
}

/// The last message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// Open the one message, and wait until its frame shows its body.
fn open_message(harness: &mut Harness) {
    let subject = ".list .ds-list-item[*|aria-posinset=\"1\"] .ds-thread .ds-thread-sub";
    for _ in 0..50 {
        if harness.rect(subject).is_some() {
            break;
        }
        harness.advance(ms(100));
    }
    let rect = harness
        .rect(subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    for _ in 0..100 {
        harness.advance(ms(100));
        if harness
            .frame(FRAME)
            .is_some_and(|frame| frame.text().contains("B12"))
        {
            return;
        }
    }
    panic!("the message's frame never showed:\n{}", harness.html());
}

/// Wait until `selector` is drawn.
fn wait_for(harness: &mut Harness, selector: &str) {
    for _ in 0..100 {
        if harness.rect(selector).is_some() {
            harness.advance(ms(300));
            return;
        }
        harness.advance(ms(100));
    }
    panic!("{selector} is never drawn:\n{}", harness.html());
}

/// Where the text of the first element matching `selector` starts, and its font size: the left
/// of its content box (inside its border and padding) and its computed `font-size`, in px.
fn text_box(harness: &Harness, selector: &str) -> (f32, f32) {
    let rect = harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    let (inset, font) = harness
        .with_doc(|doc| {
            let node = doc.get_node(doc.query_selector(selector).ok()??)?;
            let layout = node.final_layout();
            let font = node
                .primary_styles()?
                .clone_font_size()
                .computed_size()
                .px();
            Some((layout.padding.left + layout.border.left, font))
        })
        .unwrap_or_else(|| panic!("{selector} has no style"));
    (rect.origin.x.0 + inset, font)
}

/// Where the reader's body text starts, and its size: the frame's `body` content box, on screen.
fn reader_body(harness: &Harness) -> (f32, f32) {
    harness
        .with_doc(|doc| {
            let iframe = doc.get_node(doc.query_selector(FRAME).ok()??)?;
            let offset = iframe.absolute_position(0.0, 0.0);
            let sub = iframe.subdoc()?.inner();
            let body_id = sub.query_selector("body").ok()??;
            let rect = sub.get_client_bounding_rect(body_id)?;
            let body = sub.get_node(body_id)?;
            let inset = body.final_layout().padding.left + body.final_layout().border.left;
            let font = body
                .primary_styles()?
                .clone_font_size()
                .computed_size()
                .px();
            Some((offset.x + rect.x as f32 + inset, font))
        })
        .unwrap_or_else(|| panic!("the reader's frame has no body:\n{}", harness.html()))
}

/// The reader's model: the left edge of its title and of its message header, the title's size,
/// and where its body's text starts and at what size.
struct Reader {
    title_left: f32,
    title_font: f32,
    head_left: f32,
    body_left: f32,
    body_font: f32,
}

fn reader(harness: &Harness) -> Reader {
    let (title_left, title_font) = text_box(harness, ".reader-head h2 .ds-label");
    let head_left = harness
        .rect(".reader-body article.frame:last-of-type header.msg-head")
        .expect("the message's header is drawn")
        .origin
        .x
        .0;
    let (body_left, body_font) = reader_body(harness);
    Reader {
        title_left,
        title_font,
        head_left,
        body_left,
        body_font,
    }
}

fn near(what: &str, got: f32, want: f32) {
    assert!(
        (got - want).abs() <= SLACK,
        "{what} is at {got}, the reader's at {want}"
    );
}

/// Each of `rows` starts at the header's left edge; their labels start on that edge, where the
/// subject and the body do; and their values start on one x past the widest label.
fn assert_lines(harness: &Harness, page: &str, rows: &[&str], model: &Reader) {
    let mut value_lefts = Vec::new();
    for name in rows {
        let row = format!("{page} .c-props [*|data-row={name}]");
        let line = harness
            .rect(&row)
            .unwrap_or_else(|| panic!("{row} is not drawn:\n{}", harness.html()));
        near(&format!("{name}'s line"), line.origin.x.0, model.head_left);
        let label = harness
            .rect(&format!("{row} .c-line-k"))
            .unwrap_or_else(|| panic!("{row} has no label:\n{}", harness.html()));
        let value = harness
            .rect(&format!("{row} .c-line-v"))
            .unwrap_or_else(|| panic!("{row} has no value:\n{}", harness.html()));
        assert!(
            label.origin.x.0 + label.size.width.0 <= value.origin.x.0 + SLACK,
            "{name}'s label runs into its value"
        );
        near(
            &format!("{name}'s label"),
            label.origin.x.0,
            model.head_left,
        );
        value_lefts.push((*name, value.origin.x.0));
    }
    for (what, edges) in [("value's left", &value_lefts)] {
        let (first, at) = edges[0];
        for (name, x) in edges {
            assert!(
                (x - at).abs() <= SLACK,
                "{name}'s {what} is at {x}, {first}'s at {at}"
            );
        }
    }
}

/// The right edge of the first element matching `selector`.
fn right(harness: &Harness, selector: &str) -> f32 {
    let rect = harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    rect.origin.x.0 + rect.size.width.0
}

/// Press the element `selector` at its centre.
fn press(harness: &mut Harness, selector: &str) {
    let rect = harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + rect.size.width.0 / 2.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

#[test]
fn a_new_message_is_set_as_the_reader_sets_a_message() {
    let (mut harness, _dir) = open();
    open_message(&mut harness);
    let model = reader(&harness);
    near("the reader's body", model.body_left, model.head_left);

    harness.key(Key::Char('c'));
    wait_for(&mut harness, ".cpage .c-body");

    let (subject_left, subject_font) = text_box(&harness, ".cpage .c-title input");
    near("the subject", subject_left, model.title_left);
    // What you write looks like what you read: both subjects are 20.
    assert_eq!(model.title_font, 20.0, "the reader's subject size");
    assert_eq!(
        subject_font, model.title_font,
        "the composer's subject size"
    );
    // A new message from one address draws To and nothing else.
    for gone in [
        "from", "cc", "bcc", "sends", "remind", "security", "receipt",
    ] {
        assert_eq!(
            harness.count(&format!(".cpage .c-props [*|data-row={gone}]")),
            0,
            "a new message draws {gone}"
        );
    }
    assert_lines(&harness, ".cpage", &["to"], &model);
    let (body_left, body_font) = text_box(&harness, ".cpage .c-body");
    near("the body", body_left, model.body_left);
    assert_eq!(body_font, model.body_font, "the body's size");

    // Every trailing control ends on one right edge: the To row's Bcc and Send ▾.
    let edge = right(&harness, ".cpage [*|aria-label=\"Add Bcc\"]");
    let send = right(&harness, ".cpage .c-foot .c-send");
    assert!(
        (send - edge).abs() <= SLACK,
        "Send ▾ ends at {send}, Bcc at the To row's end at {edge}"
    );

    // Cc and Bcc added: three rows, one label column, every value at one x.
    press(&mut harness, ".cpage [*|aria-label=\"Add Cc\"]");
    wait_for(&mut harness, ".cpage .c-props [*|data-row=cc]");
    press(&mut harness, ".cpage [*|aria-label=\"Add Bcc\"]");
    wait_for(&mut harness, ".cpage .c-props [*|data-row=bcc]");
    assert_lines(&harness, ".cpage", &["to", "cc", "bcc"], &model);
}

#[test]
fn a_reply_is_set_as_the_reader_sets_a_message() {
    let (mut harness, _dir) = open();
    open_message(&mut harness);
    let model = reader(&harness);

    harness.key(Key::Char('r'));
    wait_for(&mut harness, ".inline-reply .c-body");

    assert_lines(&harness, ".inline-reply", &["to"], &model);
    assert_eq!(
        harness.count(".inline-reply .c-props [*|data-row=remind]"),
        0,
        "a reply draws a Remind row before one is set"
    );
    let (body_left, body_font) = text_box(&harness, ".inline-reply .c-body");
    near("the reply's body", body_left, model.body_left);
    assert_eq!(body_font, model.body_font, "the reply's body size");

    // Its bar, Send and all, is inside the window, and it is the new message's foot.
    let bar = harness
        .rect(".inline-reply .c-foot")
        .expect("the reply's bar is drawn");
    let bottom = bar.origin.y.0 + bar.size.height.0;
    assert!(
        bar.origin.y.0 >= 0.0 && bottom <= VIEW.height as f32,
        "the reply's bar runs out of the window: {bar:?}"
    );
    for name in ["Attach", "More", "Send", "Send Options"] {
        assert_eq!(
            harness.count(&format!(".inline-reply .c-foot [*|aria-label=\"{name}\"]")),
            1,
            "the reply's foot has no {name}"
        );
    }
}
