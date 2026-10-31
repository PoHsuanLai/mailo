//! Lane 3b: the other ways to answer or pass a message on. A reply with text selected in the
//! message quotes only that. From the reader's ⋯: Reply to List is offered once the message names
//! its list's address, and answers there; Edit as New opens a copy of the message to send again;
//! Redirect… sends the message itself on, unchanged.

use mail_ui_model::copy;

use super::drive::{Key, PRIMARY};
use super::look::{chips, menu_item_of};
use super::window::{Window, deliver, hours_ago, parsed, queued};

const LIST_SUBJECT: &str = "Release candidate two";
const OWN_SUBJECT: &str = "Agenda for Friday";

/// A message through a list that names where to write to it, and one the person sent Grace.
fn seeded(store: &mail_core::SqliteStore) {
    deliver(
        store,
        "list1",
        &format!(
            "From: Ada Lovelace <ada@example.test>\r\nTo: Rust <rust@lists.example.test>\r\n\
             Subject: {LIST_SUBJECT}\r\nDate: {}\r\nMessage-ID: <rc2@example.test>\r\n\
             List-Id: Rust <rust.lists.example.test>\r\n\
             List-Post: <mailto:rust@lists.example.test>\r\n\r\n\
             The second candidate is up.\r\n",
            hours_ago(1)
        ),
    );
    deliver(
        store,
        "own1",
        &format!(
            "From: Me <me@example.test>\r\nTo: Grace Hopper <grace@example.test>\r\n\
             Subject: {OWN_SUBJECT}\r\nDate: {}\r\nMessage-ID: <agenda@example.test>\r\n\r\n\
             First the budget, then the hiring plan.\r\n",
            hours_ago(2)
        ),
    );
}

/// Open the reader's ⋯.
fn more(window: &mut Window) {
    window.click(&format!(
        ".reader-head [*|aria-label=\"{}\"]",
        copy::aria::MORE
    ));
    window.until("the ⋯ opens", |h| h.count(".ds-menu .ds-menu-item") > 0);
}

#[test]
fn reply_to_list_answers_at_the_lists_address() {
    let mut window = Window::open(seeded);
    window.open_subject(LIST_SUBJECT);
    more(&mut window);
    window.menu_item(copy::menus::REPLY_TO_LIST);
    window.until("Reply to List opens a reply", |h| h.count(".c-body") == 1);
    let to = ".c-props [*|data-row=to]";
    window.until("the list is in To", |h| {
        chips(h, to) == 1 && h.text_of(to).is_some_and(|to| to.contains("rust"))
    });
    window.press(&[PRIMARY], Key::Enter, 1);
    window.until("the reply is sent", |h| h.count(".ds-send-pill") == 1);

    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    assert_eq!(
        sent[0].rcpt_to,
        ["rust@lists.example.test"],
        "only the list"
    );
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, format!("Re: {LIST_SUBJECT}"));
    assert_eq!(mail.in_reply_to.as_deref(), Some("rc2@example.test"));
    assert!(
        sent[0].raw.contains("> The second candidate is up."),
        "the message is quoted:\n{}",
        sent[0].raw
    );

    // A message that names no list address is not offered it.
    window.open_subject(OWN_SUBJECT);
    more(&mut window);
    assert!(
        menu_item_of(&window.harness, copy::menus::REDIRECT).is_some(),
        "the ⋯ has no Redirect…"
    );
    assert!(
        menu_item_of(&window.harness, copy::menus::REPLY_TO_LIST).is_none(),
        "Reply to List offered where the message names no list"
    );
}

#[test]
fn edit_as_new_opens_a_copy_to_send_again_and_redirect_sends_the_message_on() {
    let mut window = Window::open(seeded);
    window.open_subject(OWN_SUBJECT);
    more(&mut window);
    window.menu_item(copy::menus::EDIT_AS_NEW);
    window.until("Edit as New opens a copy", |h| {
        h.text_of(".c-body")
            .is_some_and(|body| body.contains("First the budget"))
    });
    let to = ".c-props [*|data-row=to]";
    window.until("Grace is in To again", |h| {
        chips(h, to) == 1 && h.text_of(to).is_some_and(|to| to.contains("Grace"))
    });
    window.press(&[PRIMARY], Key::Enter, 1);
    window.until("the copy is sent", |h| h.count(".ds-send-pill") == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    assert_eq!(sent[0].rcpt_to, ["grace@example.test"]);
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, OWN_SUBJECT, "no Re: or Fwd:");
    assert_eq!(mail.in_reply_to, None, "it answers nothing");
    assert!(
        mail.text
            .as_deref()
            .is_some_and(|text| text.contains("First the budget, then the hiring plan.")),
        "{:?}",
        mail.text
    );

    // Redirect…: the message itself, to Alan, from a field where the ⋯ hung.
    window.open_subject(OWN_SUBJECT);
    more(&mut window);
    window.menu_item(copy::menus::REDIRECT);
    window.until("Redirect… asks whom", |h| h.count(".redirect input") == 1);
    window.type_text("alan@example.test\n");
    window.until("the redirect is said", |h| {
        h.text_of(".ds-toast-body")
            .is_some_and(|said| said.contains("alan@example.test"))
    });
    let sent = queued(&window.store);
    let redirect = sent
        .iter()
        .find(|one| one.rcpt_to == ["alan@example.test"])
        .expect("a submission to Alan");
    assert!(
        redirect.raw.starts_with("Resent-Date: "),
        "{}",
        redirect.raw
    );
    assert!(
        redirect.raw.contains("Message-ID: <agenda@example.test>")
            && redirect
                .raw
                .contains("To: Grace Hopper <grace@example.test>"),
        "the original is not as it came:\n{}",
        redirect.raw
    );
}

const SELECTED_SUBJECT: &str = "Two thoughts";

/// A plain message of two paragraphs, the first a line on its own.
fn two_thoughts(store: &mail_core::SqliteStore) {
    deliver(
        store,
        "two1",
        &format!(
            "From: Ada Lovelace <ada@example.test>\r\nTo: Me <me@example.test>\r\n\
             Subject: {SELECTED_SUBJECT}\r\nDate: {}\r\nMessage-ID: <two@example.test>\r\n\r\n\
             Alpha comes first\r\n\r\nBeta comes after\r\n",
            hours_ago(1)
        ),
    );
}

#[test]
fn a_reply_with_text_selected_in_the_message_quotes_only_that() {
    use ds::prelude::{Point, Px};
    use ds_harness::Input;
    let mut window = Window::open(two_thoughts);
    window.open_subject(SELECTED_SUBJECT);
    let frame = ".reader article.frame iframe";
    window.until("the message is drawn", |h| {
        h.frame(frame)
            .is_some_and(|inside| inside.text().contains("Beta"))
    });
    // A drag along the first line, from just inside the body's margin.
    let rect = window.harness.rect(frame).expect("the frame's box");
    let y = rect.origin.y.0 + 18.0 + 11.0;
    let from = Point {
        x: Px(rect.origin.x.0 + 19.0),
        y: Px(y),
    };
    let to = Point {
        x: Px(rect.origin.x.0 + 18.0 + 200.0),
        y: Px(y),
    };
    window.harness.send(Input::pointer_move(from));
    window.harness.send(Input::pointer_down(from));
    window.harness.advance(super::window::ms(50));
    for step in 1..=10 {
        let part = step as f32 / 10.0;
        window.harness.send(Input::pointer_move(Point {
            x: Px(from.x.0 + (to.x.0 - from.x.0) * part),
            y: Px(y),
        }));
        window.harness.advance(super::window::ms(16));
    }
    window.harness.send(Input::pointer_up(to));
    window.harness.advance(super::window::ms(100));

    window.until("the drag selects the first line", |h| {
        h.with_doc(|doc| {
            let found = doc.query_selector(frame).ok().flatten()?;
            let sub = doc.get_node(found)?.subdoc()?.inner();
            sub.get_selected_text()
        })
        .is_some_and(|text| text.contains("Alpha"))
    });
    window.press(&[], Key::Char('r'), 1);
    window.until("r opens a reply", |h| h.count(".inline-reply .c-body") == 1);
    window.press(&[PRIMARY], Key::Enter, 1);
    window.until("the reply is sent", |h| h.count(".ds-send-pill") == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    let raw = &sent[0].raw;
    assert!(
        raw.contains("> Alpha"),
        "the selection is not quoted:\n{raw}"
    );
    assert!(
        !raw.contains("Beta"),
        "more than the selection is quoted:\n{raw}"
    );
}
