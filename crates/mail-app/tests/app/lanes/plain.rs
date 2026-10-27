//! Plain text: with Settings › Composing › Message format on Plain text, a new message goes out as
//! its text alone, and the composer offers no formatting; and a reply to plain-text mail answers
//! in plain text even where new mail is written rich, as Reply in the Same Format says by default.

use super::drive::{Drive, Key, PRIMARY};
use super::window::{INBOX, Window, parsed, queued};

/// The composer's To field.
const TO: &str = ".c-props [*|data-row=to] .c-pin input";

/// The window, its `settings.toml` first changed by `edit`.
fn open_with(edit: impl FnOnce(&mut mail_ui::settings::MailSettings) + 'static) -> Window {
    Window::open_with(
        chrono::Utc::now(),
        |_| {},
        move |dir, contexts| {
            mail_ui::settings::change(&mail_ui::settings::root_for(&dir.join("config")), edit)
                .expect("the setting is written");
            contexts
        },
    )
}

/// The submission's bytes, lowercased, with no HTML part in them.
fn assert_text_alone(raw: &str) {
    let lower = raw.to_ascii_lowercase();
    assert!(lower.contains("content-type: text/plain"), "{raw}");
    assert!(
        !lower.contains("text/html"),
        "an HTML part went out:\n{raw}"
    );
    assert!(!lower.contains("multipart/alternative"), "{raw}");
}

#[test]
fn a_new_message_in_plain_text_goes_as_its_text_alone_with_no_formatting_offered() {
    let mut window = open_with(|settings| {
        settings.compose.format = mail_ui::settings::MessageFormat::Plain;
    });
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(TO);
    window.type_text("ada@example.test");
    window.press(&[], Key::Enter, 1);
    window.until("Ada is in To", |h| {
        h.count(".c-props [*|data-row=to] .ds-chip") == 1
    });
    window.click(".c-title input");
    window.type_text("Lunch");
    window.click(".c-body");
    window.type_text("Thursday at noon?");

    // A selection offers no formatting bubble: there is nothing plain text can carry of it.
    window.press(&[Key::Shift], Key::Left, 4);
    window.harness.advance(super::window::ms(200));
    assert_eq!(window.harness.count(".bubble"), 0, "a bubble on plain text");
    window.press(&[], Key::End, 1);

    let before = queued(&window.store).len();
    window.press(&[PRIMARY], Key::Enter, 1);
    window.until("the message is sent", |h| h.count(".ds-send-pill") == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), before + 1, "not one submission");
    let mail = parsed(&sent[before].raw);
    assert_eq!(mail.subject, "Lunch");
    assert_eq!(mail.html, None);
    assert!(
        mail.text
            .as_deref()
            .is_some_and(|text| text.contains("Thursday at noon?")),
        "{:?}",
        mail.text
    );
    assert_text_alone(&sent[before].raw);
}

#[test]
fn a_reply_to_plain_text_mail_answers_in_plain_text_where_new_mail_is_rich() {
    let mut window = Window::open(|_| {});
    let subject = INBOX[0].1;
    window.open_subject(subject);
    window.press(&[], Key::Char('r'), 1);
    window.until("r opens a reply", |h| h.count(".inline-reply .c-body") == 1);
    window.until("the reply's body has the keyboard", |h| {
        h.is_focused(".inline-reply .c-body")
    });
    window.type_text("**Yes**, see you there.");
    let before = queued(&window.store).len();
    window.press(&[PRIMARY], Key::Enter, 1);
    window.until("the reply is sent", |h| h.count(".ds-send-pill") == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), before + 1, "not one submission");
    let mail = parsed(&sent[before].raw);
    assert_eq!(mail.subject, format!("Re: {subject}"));
    assert_eq!(mail.html, None, "the plain original was answered rich");
    assert_text_alone(&sent[before].raw);
}
