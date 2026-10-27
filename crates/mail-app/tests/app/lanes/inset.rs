//! A message's words keep a gutter from the frame's edge, whatever the sender's `<body>` says:
//! a notice whose body is `margin:0;padding:0` is read 18 px in, as a plain-text message is, so
//! every message lines up with the reader's header; and a sender's ground still reaches the
//! frame's edges.

use ds::prelude::{Point, Px};
use ds_harness::Harness;

use super::drive::Drive;
use super::window::{Window, deliver, ms};

/// A security notice laid out as a bulk sender's tool lays one out: `multipart/alternative`, its
/// HTML a `<body style="margin:0;padding:0">` holding flush paragraphs and no centring table.
const NOTICE: &str = include_str!("../../../../mail-core/tests/fixtures/flush-notice.eml");
const NOTICE_SUBJECT: &str = "Please rotate a key you used";
const PLAIN_SUBJECT: &str = "Plain words";
const GROUND_SUBJECT: &str = "A newsletter on a blue ground";
/// The newsletter's ground, `#1d4e89`.
const GROUND: [u8; 3] = [0x1d, 0x4e, 0x89];
/// The open message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

fn letters(store: &mail_core::SqliteStore) {
    deliver(store, "notice", NOTICE);
    let date = chrono::Utc::now().to_rfc2822();
    deliver(
        store,
        "plain",
        &format!(
            "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: {PLAIN_SUBJECT}\r\n\
             Date: {date}\r\nMessage-ID: <plain-inset@example.test>\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\r\nFirst line.\r\n"
        ),
    );
    deliver(
        store,
        "ground",
        &format!(
            "From: news@example.test\r\nTo: me@example.test\r\nSubject: {GROUND_SUBJECT}\r\n\
             Date: {date}\r\nMessage-ID: <ground-inset@example.test>\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n\
             <html><head><style>div:not(#z){{padding:0!important;width:100%!important}}</style>\
             </head><body bgcolor=\"#1d4e89\" style=\"margin:0;padding:0\">\
             <table width=\"100%\" bgcolor=\"#1d4e89\"><tr><td align=\"center\">\
             <table width=\"300\" bgcolor=\"#ffffff\"><tr><td id=\"story\">The week's news</td>\
             </tr></table></td></tr></table></body></html>\r\n"
        ),
    );
}

/// How far in from the frame's left edge the frame's first element matching `selector` starts.
fn indent(harness: &Harness, selector: &str) -> Option<f32> {
    harness.with_doc(|doc| {
        // A selector of this file's that does not parse finds nothing, as a missing one does.
        let iframe = doc.query_selector(FRAME).ok()??;
        let sub = doc.get_node(iframe)?.subdoc()?.inner();
        // The same for the one asked about.
        let found = sub.query_selector(selector).ok()??;
        Some(sub.get_client_bounding_rect(found)?.x as f32)
    })
}

/// Where the first run of text of the frame's `selector` starts, in from the frame's left edge.
fn text_indent(harness: &Harness, selector: &str) -> Option<f32> {
    harness.with_doc(|doc| {
        // As in `indent`.
        let iframe = doc.query_selector(FRAME).ok()??;
        let sub = doc.get_node(iframe)?.subdoc()?.inner();
        // As in `indent`.
        let found = sub.query_selector(selector).ok()??;
        let node = sub.get_node(found)?;
        let layout = node.final_layout();
        let at = sub.get_client_bounding_rect(found)?.x as f32;
        Some(at + layout.padding.left + layout.border.left)
    })
}

/// Open `subject`, and wait until its frame's `selector` is laid out in a frame its own width.
fn open(window: &mut Window, subject: &str, selector: &str) {
    window.open_subject(subject);
    let what = format!("{subject:?}'s words are drawn");
    window.until(&what, |h| indent(h, selector).is_some());
    // The pointer over the frame once, as a person's is, so its layout is the settled one.
    let frame = window.harness.rect(FRAME).expect("the frame");
    window.harness.pointer_move(Point {
        x: Px(frame.origin.x.0 + frame.size.width.0 / 2.0),
        y: Px(frame.origin.y.0 + frame.size.height.0 - 8.0),
    });
    window.harness.advance(ms(300));
}

#[test]
fn a_flush_message_is_read_with_the_same_gutter_as_a_plain_one() {
    let mut window = Window::open(letters);

    open(&mut window, NOTICE_SUBJECT, "p");
    let html = indent(&window.harness, "p").expect("the first paragraph");
    assert!(
        html >= 16.0,
        "the first paragraph starts {html} px from the frame's edge"
    );
    let width = window.harness.rect(FRAME).expect("the frame").size.width.0;
    let p = window
        .harness
        .frame(FRAME)
        .and_then(|frame| frame.width("p"))
        .expect("the first paragraph")
        .0;
    assert!(
        html + p <= width - 16.0 + 0.5,
        "the first paragraph runs to {} px of a {width} px frame",
        html + p
    );

    open(&mut window, PLAIN_SUBJECT, "body");
    // The lines are the inset's text where there is one, else the body's own.
    let plain = text_indent(&window.harness, ".mailo-inset")
        .or_else(|| text_indent(&window.harness, "body"))
        .expect("the plain body");
    assert!(
        (plain - html).abs() < 0.5,
        "a plain message starts {plain} px in, an HTML one {html} px"
    );
}

#[test]
fn a_sender_s_ground_still_reaches_the_frame_s_edges() {
    let mut window = Window::open(letters);
    open(&mut window, GROUND_SUBJECT, "#story");
    let frame = window.harness.rect(FRAME).expect("the frame");
    let shot = window.harness.render().expect("the window renders");
    let scale = shot.width() as f32 / window.harness.window_size().width as f32;
    let at = |x: f32, y: f32| {
        let pixel = shot.get_pixel((x * scale) as u32, (y * scale) as u32);
        [pixel[0], pixel[1], pixel[2]]
    };
    let top = frame.origin.y.0 + 2.0;
    let left = frame.origin.x.0 + 2.0;
    let right = frame.origin.x.0 + frame.size.width.0 - 3.0;
    for (name, x, y) in [
        ("top left", left, top),
        ("top right", right, top),
        ("left, halfway down", left, top + 60.0),
    ] {
        let seen = at(x, y);
        let near = seen
            .iter()
            .zip(GROUND)
            .all(|(seen, ground)| seen.abs_diff(ground) <= 3);
        assert!(
            near,
            "the frame's {name} is {seen:?}, not the sender's ground"
        );
    }
}
