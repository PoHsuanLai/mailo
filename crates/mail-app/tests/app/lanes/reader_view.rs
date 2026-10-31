//! Lane 0, the reader's own choices: ⌘+, ⌘− and ⌘0 draw a message's text larger, smaller and at
//! its own size, and the size is kept in the settings; View Source and Show All Headers show the
//! message as it came; and a conversation opened under "mark as read: never" stays unread, as
//! one passed over before "after a delay" is up does.

use ds_harness::Harness;
use mail_core::Store;
use mail_domain::ReadState;
use mail_ui_model::copy;
use mail_ui_model::settings::{MailSettings, MarkRead, MarkReadDelay, TextZoom};

use super::drive::{Key, PRIMARY};
use super::look::conversation;
use super::window::{Window, deliver, ms};

const NEWSLETTER: &str = "A short letter";
const OTHER: &str = "Another letter";

/// The last message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// A distinctive header line, which only the message's own bytes carry.
const MAILER: &str = "X-Mailer: Lane Mailer 7";

fn letters(store: &mail_core::SqliteStore) {
    let letters = [
        (
            "view0",
            NEWSLETTER,
            "<view0@example.test>",
            "<html><body><p><span id=\"word\">Hello there</span></p></body></html>",
        ),
        (
            "view1",
            OTHER,
            "<view1@example.test>",
            "<html><body><p>Something else</p></body></html>",
        ),
    ];
    for (n, (uidl, subject, id, body)) in letters.iter().enumerate() {
        let raw = format!(
            "To: me@example.test\r\nFrom: ada@example.test\r\nSubject: {subject}\r\n\
             Message-ID: {id}\r\n{MAILER}\r\nDate: {}\r\nMIME-Version: 1.0\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n{body}\r\n",
            (chrono::Utc::now() - chrono::Duration::minutes(10 - n as i64)).to_rfc2822()
        );
        deliver(store, uidl, &raw);
    }
}

/// The window over the seeded inbox and [`letters`], its settings first changed by `edit`.
fn open(edit: impl FnOnce(&mut MailSettings) + 'static) -> Window {
    Window::open_with(chrono::Utc::now(), letters, move |dir, contexts| {
        let root = mail_ui_model::settings::root_for(&dir.join("config"));
        mail_ui_model::settings::change(&root, edit).expect("the settings are written");
        contexts
    })
}

/// Wait for the text size to be in the settings file: it is written a moment after the last
/// press, off the drawing thread.
fn kept(window: &mut Window, zoom: u16) {
    let root = mail_ui_model::settings::root_for(&window.dirs().config);
    window.until(&format!("text size {zoom} is written"), move |_| {
        mail_ui_model::settings::load(&root).reading.text_zoom == TextZoom(zoom)
    });
}

/// What `read` measures the open message as, once it has stopped changing.
fn settled(window: &mut Window, read: impl Fn(&Harness) -> Option<f32>) -> f32 {
    let (mut was, mut same) = (read(&window.harness), 0);
    for _ in 0..100 {
        window.harness.advance(ms(150));
        let now = read(&window.harness);
        same = if now == was { same + 1 } else { 0 };
        if same == 4 {
            return now.unwrap_or_else(|| panic!("never drawn:\n{}", window.harness.html()));
        }
        was = now;
    }
    panic!("never settled:\n{}", window.harness.html());
}

/// The width of the frame's one word, as drawn.
fn word(h: &Harness) -> Option<f32> {
    h.frame(FRAME).and_then(|f| f.width("#word")).map(|px| px.0)
}

#[test]
fn bigger_smaller_and_actual_size_draw_the_message_s_text_and_are_kept() {
    let mut window = open(|_| {});
    window.open_subject(NEWSLETTER);
    window.until("the letter is drawn", |h| {
        h.frame(FRAME).is_some_and(|f| f.count("#word") == 1)
    });
    let own = settled(&mut window, word);

    // ⌘+ (pressed as ⌘=, as a Mac takes it): a rung larger, in the frame and in the file.
    window.press(&[PRIMARY], Key::Char('='), 1);
    let bigger = settled(&mut window, word);
    assert!(
        (bigger / own - 1.1).abs() < 0.03,
        "⌘+ drew the word {own} -> {bigger}"
    );
    kept(&mut window, 110);

    // ⌘− twice: a rung under its own size.
    window.press(&[PRIMARY], Key::Char('-'), 2);
    let smaller = settled(&mut window, word);
    assert!(
        (smaller / own - 0.9).abs() < 0.03,
        "⌘− twice drew the word {own} -> {smaller}"
    );
    kept(&mut window, 90);

    // ⌘0: its own size again.
    window.press(&[PRIMARY], Key::Char('0'), 1);
    kept(&mut window, 100);
    let actual = settled(&mut window, word);
    assert!(
        (actual - own).abs() < 0.5,
        "⌘0 drew the word {actual}, not {own}"
    );
}

#[test]
fn view_source_and_show_all_headers_show_the_message_as_it_came() {
    let mut window = open(|_| {});
    window.open_subject(NEWSLETTER);
    let thread = conversation(&window.store, NEWSLETTER).id;
    let message = *window
        .store
        .thread(thread)
        .expect("the conversation")
        .messages
        .last()
        .expect("a message");
    let raw = mail_core::export::raw_message(&window.store, message)
        .expect("the store reads")
        .expect("its bytes are here");
    let raw = String::from_utf8(raw).expect("the lane's mail is text");

    // View Source from the reader's ⋯: the stored bytes whole, headers and body.
    window.click(&format!(
        ".reader-head button[*|aria-label=\"{}\"]",
        copy::aria::MORE
    ));
    window.menu_item(copy::menus::VIEW_SOURCE);
    window.until("the source is shown", |h| h.count(".raw-text") == 1);
    assert_eq!(
        window.text(".raw-sheet .ds-label").trim(),
        copy::titles::MESSAGE_SOURCE
    );
    assert_eq!(window.text(".raw-text"), raw);
    assert!(raw.contains(MAILER) && raw.contains("id=\"word\""), "{raw}");

    // Esc closes it, and the conversation is still open behind it.
    window.press(&[], Key::Escape, 1);
    window.until("Esc closes the sheet", |h| h.count(".raw-sheet") == 0);
    assert!(window.text(".reader").contains(NEWSLETTER));

    // ⇧⌘H: the header lines alone, up to the empty line before the body.
    window.press(&[PRIMARY, Key::Shift], Key::Char('h'), 1);
    window.until("the headers are shown", |h| h.count(".raw-text") == 1);
    let headers = window.text(".raw-text");
    let end = raw.find("\r\n\r\n").expect("a body") + 2;
    assert_eq!(headers, raw[..end]);
    assert!(!headers.contains("id=\"word\""), "{headers}");
    let done = ".raw-sheet .sheet-actions button:last-child";
    assert_eq!(window.text(done).trim(), copy::controls::DONE);
    window.click(done);
    window.until("Done closes the sheet", |h| h.count(".raw-sheet") == 0);
}

#[test]
fn a_conversation_opened_under_never_stays_unread() {
    let mut window = open(|settings| settings.reading.mark_read = MarkRead::Never);
    assert_eq!(
        conversation(&window.store, NEWSLETTER).read,
        ReadState::Unread
    );
    window.open_subject(NEWSLETTER);
    window.harness.advance(ms(15_000));
    assert_eq!(
        conversation(&window.store, NEWSLETTER).read,
        ReadState::Unread,
        "opening it marked it read"
    );
    assert!(window.text(".reader").contains(NEWSLETTER));
}

#[test]
fn after_a_delay_marks_what_stays_shown_and_not_what_was_passed_over() {
    let mut window = open(|settings| {
        settings.reading.mark_read = MarkRead::AfterDelay;
        settings.reading.mark_read_delay = MarkReadDelay::Two;
    });
    let read = |window: &Window, subject: &str| conversation(&window.store, subject).read;

    // Passed over a second in: left unread, however long after.
    window.open_subject(NEWSLETTER);
    window.harness.advance(ms(1_000));
    assert_eq!(
        read(&window, NEWSLETTER),
        ReadState::Unread,
        "before its time"
    );
    window.open_subject(OTHER);
    window.harness.advance(ms(1_200));
    assert_eq!(read(&window, OTHER), ReadState::Unread, "the next, early");

    // The one that stays shown is read once its two seconds are up.
    window.until_stored("the one shown is read", |store| {
        conversation(store, OTHER).read == ReadState::Read
    });
    window.harness.advance(ms(5_000));
    assert_eq!(
        read(&window, NEWSLETTER),
        ReadState::Unread,
        "the one passed over was marked read"
    );
}
