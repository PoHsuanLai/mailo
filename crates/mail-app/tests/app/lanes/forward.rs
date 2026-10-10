//! Lane 6: forwarding a message that carries a PDF. The reader lists the attachment; `f` forwards
//! the message with its files, and Forward as attachment encloses the whole message.

use ds_harness::Query;

use super::drive::{Key, PRIMARY};
use super::window::{Window, deliver, hours_ago, panel_row, panel_settled, parsed, queued};

const SUBJECT: &str = "Minutes of the board";
const PDF: &str = "%PDF-1.4\n% the minutes\n";
const SEND: &str = ".c-foot [*|aria-label=\"Send\"]";
const TO: &str = ".c-props [*|data-row=to] .c-pin input";

fn with_pdf(store: &mail_store::SqliteStore) {
    let raw = format!(
        "From: Edsger Dijkstra <edsger@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {SUBJECT}\r\nDate: {}\r\nMessage-ID: <minutes1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nThe minutes are attached.\r\n\
         --mix\r\nContent-Type: application/pdf; name=\"minutes.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"minutes.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--mix--\r\n",
        hours_ago(0),
        base64(PDF.as_bytes())
    );
    deliver(store, "minutes1", &raw);
}

/// Standard base64, for the seeded part.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The message open, its attachment listed.
fn opened() -> Window {
    let mut window = Window::open(with_pdf);
    window.open_subject(SUBJECT);
    window.until("the reader lists the PDF", |h| {
        h.text_of(".reader .attachments")
            .is_some_and(|rows| rows.contains("minutes.pdf"))
    });
    window
}

/// Address the open forward to Grace and send it.
fn send_to_grace(window: &mut Window) {
    window.click(TO);
    window.type_text("grace@example.test\n");
    window.until("Grace is in To", |h| {
        h.count(".c-props [*|aria-label=\"Remove Grace Hopper\"]") == 1
    });
    window.click(SEND);
    window.until("the forward is sent", |h| h.count(".ds-send-pill") == 1);
}

#[test]
#[ignore = "gap: f forwards the text alone; the original's attachments are not carried (Draft::forward_of leaves attachments empty)"]
fn f_forwards_the_message_with_its_pdf() {
    let mut window = opened();
    window.press(&[], Key::Char('f'), 1);
    window.until("f opens a forward", |h| h.count(".cpage .c-body") == 1);
    window.until("the forward carries the PDF", |h| {
        h.text_of(".c-props [*|data-row=attached]")
            .is_some_and(|row| row.contains("minutes.pdf"))
    });
    send_to_grace(&mut window);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1);
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, format!("Fwd: {SUBJECT}"));
    assert_eq!(mail.attachments.len(), 1);
    assert_eq!(mail.attachments[0].name, "minutes.pdf");
    assert_eq!(mail.attachments[0].bytes, PDF.as_bytes());
}

#[test]
fn f_forwards_the_text_under_a_header_and_forward_as_attachment_encloses_the_message() {
    let mut window = opened();

    // f: a forward of the open message, its text under a header block.
    window.press(&[], Key::Char('f'), 1);
    window.until("f opens a forward", |h| h.count(".cpage .c-body") == 1);
    let body = window.text(".cpage .c-body");
    assert!(
        body.contains("Forwarded message") && body.contains("The minutes are attached."),
        "{body}"
    );
    send_to_grace(&mut window);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one forward");
    assert_eq!(sent[0].rcpt_to, ["grace@example.test"]);
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, format!("Fwd: {SUBJECT}"));
    assert!(
        mail.text
            .as_deref()
            .is_some_and(|text| text.contains("From: Edsger Dijkstra")),
        "{:?}",
        mail.text
    );

    // Forward as attachment, from the search panel's commands: the message itself enclosed.
    window.open_subject(SUBJECT);
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K opens the panel", |h| h.count(".spotlight input") == 1);
    window.type_text("Forward as attachment");
    window.until("the panel answers what was typed", |h| {
        panel_settled(h) && panel_row(h, "Forward as attachment").is_some()
    });
    let n = panel_row(&window.harness, "Forward as attachment").unwrap_or_default();
    window.click(&format!(".spotlight-rows .ds-menu > :nth-child({n})"));
    window.until("the forward encloses the message", |h| {
        h.text_of(".c-props [*|data-row=attached]")
            .is_some_and(|row| row.contains(".eml"))
    });
    send_to_grace(&mut window);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 2, "the second forward was not queued");
    let enclosed = sent
        .iter()
        .map(|one| parsed(&one.raw))
        .find(|mail| !mail.attachments.is_empty())
        .expect("a forward with an attachment");
    assert_eq!(enclosed.attachments.len(), 1);
    let part = &enclosed.attachments[0];
    assert_eq!(part.mime, "message/rfc822");
    assert_eq!(part.name, format!("{SUBJECT}.eml"));
    let inner = String::from_utf8_lossy(&part.bytes);
    assert!(
        inner.contains("Message-ID: <minutes1@example.test>") && inner.contains("minutes.pdf"),
        "the enclosed message is not the original:\n{inner}"
    );
}
