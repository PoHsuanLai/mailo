//! A newsletter whose images and labels float beside its words is laid out and read, as any
//! other message is. Blitz marks a floated element for the float layout; built without that
//! layout, the line breaker handed such a paragraph returned to the element for ever, and the
//! window stopped answering the moment the message was laid out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::window::{Window, deliver};

const SUBJECT: &str = "This week's picks";
/// The open message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";
/// Longer than any lane takes to open a message; a window looping in layout never gets there.
const DEADLINE: Duration = Duration::from_secs(120);

fn letters(store: &mail_core::SqliteStore) {
    let date = chrono::Utc::now().to_rfc2822();
    deliver(
        store,
        "floats",
        &format!(
            "From: news@example.test\r\nTo: me@example.test\r\nSubject: {SUBJECT}\r\n\
             Date: {date}\r\nMessage-ID: <floats@example.test>\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n\
             <html><body><table width=\"100%\"><tr><td>\
             <p id=\"pick\"><img align=\"left\" width=\"40\" height=\"40\" alt=\"\"> A jacket \
             <span style=\"float:right\">$120</span> in wool, lined, with two pockets.</p>\
             </td></tr></table></body></html>\r\n"
        ),
    );
}

#[test]
fn a_message_with_floated_images_is_laid_out_and_read() {
    // A window caught in layout never returns to this test, so the deadline fails it instead.
    let opened = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&opened);
    std::thread::spawn(move || {
        std::thread::sleep(DEADLINE);
        if !seen.load(Ordering::Relaxed) {
            eprintln!("the message with floats was not laid out in {DEADLINE:?}");
            std::process::exit(1);
        }
    });

    let mut window = Window::open(letters);
    window.open_subject(SUBJECT);
    window.until("the floated message's words are drawn", |h| {
        h.with_doc(|doc| {
            // A selector of this file's that does not parse finds nothing, as a missing one does.
            let iframe = doc.query_selector(FRAME).ok()??;
            let sub = doc.get_node(iframe)?.subdoc()?.inner();
            // The same for the paragraph.
            let pick = sub.query_selector("#pick").ok()??;
            Some(sub.get_client_bounding_rect(pick)?.height > 0.0)
        })
        .unwrap_or(false)
    });
    opened.store(true, Ordering::Relaxed);
}
