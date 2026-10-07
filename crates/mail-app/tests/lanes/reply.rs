//! Lane 3: a reply's round trip. Open a message, press r, write a formatted answer above the
//! quote and send it; the answer to it comes back into the same conversation, and Reply all on
//! that one addresses everyone on it.

use ds_harness::Query;

use super::drive::{Drive, Key};
use super::window::{INBOX, Window, hours_ago, parsed, queued};

/// The inline reply under the open conversation.
const REPLY: &str = ".inline-reply";

#[test]
fn a_reply_goes_out_quoted_its_answer_threads_back_and_reply_all_answers_everyone() {
    let mut window = Window::open(|_| {});
    let subject = INBOX[0].1;
    window.open_subject(subject);

    // r: the reply opens under the conversation, Ada in To, her message quoted and folded
    // under a line that says who wrote it.
    window.press(&[], Key::Char('r'), 1);
    window.until("r opens a reply", |h| {
        h.count(&format!("{REPLY} .c-body")) == 1
    });
    assert_eq!(
        window
            .harness
            .count(&format!("{REPLY} [*|aria-label=\"Remove Ada Lovelace\"]")),
        1,
        "Ada is not in To: {}",
        window.text(&format!("{REPLY} .c-props"))
    );
    let quote = window.text(&format!("{REPLY} .o-rq"));
    assert!(
        quote.contains("Ada Lovelace") && quote.contains("show quoted text"),
        "no folded quote naming Ada: {quote:?}"
    );

    // The answer, written above the quote, with a bold word typed as Markdown.
    window.until("the reply's body has the keyboard", |h| {
        h.is_focused(&format!("{REPLY} .c-body"))
    });
    window.type_text("**Yes**, I land at nine.");
    assert_eq!(
        window.text(&format!("{REPLY} .c-body .m-b")),
        "Yes",
        "the bold did not close"
    );
    window.press(&[Key::Ctrl], Key::Enter, 1);
    window.until("the reply is sent", |h| h.count(".ds-send-pill") == 1);

    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    assert_eq!(sent[0].rcpt_to, ["ada@example.test"]);
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, format!("Re: {subject}"));
    assert_eq!(mail.in_reply_to.as_deref(), Some("seed0@example.test"));
    assert_eq!(mail.references, ["seed0@example.test"]);
    // The text part reads as an answer, then who wrote what it answers, then that, quoted.
    let raw = sent[0].raw.as_str();
    let answer = raw
        .find("Yes, I land at nine.")
        .expect("the answer is in the text");
    let wrote = raw
        .find(", Ada Lovelace wrote:\r\n> The body of")
        .unwrap_or_else(|| panic!("no attribution line before the quote:\n{raw}"));
    let on = raw[..wrote]
        .rfind("\r\nOn ")
        .expect("the attribution starts with On");
    assert!(answer < on, "not answer, then quote:\n{raw}");
    let html = mail.html.clone().unwrap_or_default();
    assert!(html.contains("<strong>Yes</strong>"), "{html}");
    assert!(
        html.contains("<blockquote"),
        "the HTML part quotes nothing:\n{html}"
    );
    let ours = mail
        .rfc_message_id
        .clone()
        .expect("the reply has a Message-ID");

    // Ada answers, copying Grace. It arrives as a sync stores it, and joins the conversation.
    window.arrives(
        "answer1",
        &format!(
            "From: Ada Lovelace <ada@example.test>\r\nTo: Me <me@example.test>\r\n\
             Cc: Grace Hopper <grace@example.test>\r\nSubject: Re: Re: {subject}\r\n\
             Date: {}\r\nMessage-ID: <answer1@example.test>\r\nIn-Reply-To: <{ours}>\r\n\
             References: <seed0@example.test> <{ours}>\r\n\r\nSee you at the gate.\r\n",
            hours_ago(0)
        ),
    );
    window.until("the answer shows in the open conversation", |h| {
        h.count(".reader article.frame") == 2
            && h.text_of(".reader")
                .is_some_and(|reader| reader.contains("Cc: Grace Hopper"))
    });
    assert_eq!(
        window.subjects().len(),
        INBOX.len(),
        "the answer started a conversation of its own: {:?}",
        window.subjects()
    );

    // Reply all on it: Ada in To, Grace in Cc, and never me.
    window.press(&[], Key::Char('a'), 1);
    window.until("a opens a reply to all", |h| {
        h.count(&format!("{REPLY} .c-body")) == 1
    });
    let props = format!("{REPLY} .c-props");
    window.until("Grace is in Cc", |h| {
        h.text_of(&format!("{props} [*|data-row=cc]"))
            .is_some_and(|cc| cc.contains("Grace Hopper"))
    });
    let to = window.text(&format!("{props} [*|data-row=to]"));
    assert!(to.contains("Ada Lovelace"), "Ada is not in To: {to:?}");
    assert_eq!(
        window
            .harness
            .count(&format!("{props} [*|aria-label=\"Remove Me\"]")),
        0,
        "I am among the recipients: {}",
        window.text(&props)
    );
}
