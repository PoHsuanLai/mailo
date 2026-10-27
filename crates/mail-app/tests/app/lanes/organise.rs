//! Lane 16: Organise by Conversation. The list's ⋯ turns it off for the inbox: each message is a
//! row of its own, a reply beside the message it answers, and opening one shows that message
//! alone. The choice is kept in `settings.toml` for the inbox and for no other place, and turning
//! it back on lists the conversation once again.

use mail_app::settings::{load, root_for};
use mail_app::ui::copy;
use mail_core::Store as _;

use super::drive::Key;
use super::window::{Window, account, deliver, hours_ago};

const LUNCH: &str = "Lunch on Thursday";
const REPLY: &str = "Re: Lunch on Thursday";

/// Alan's answer to his own lunch invitation, newer than everything seeded.
fn answer(store: &mail_core::SqliteStore) {
    let raw = format!(
        "From: Alan Turing <alan@example.test>\r\nTo: Me <me@example.test>\r\nSubject: {REPLY}\r\n\
         Date: {}\r\nMessage-ID: <lunch-reply@example.test>\r\nIn-Reply-To: <seed2@example.test>\r\n\
         References: <seed2@example.test>\r\n\r\nNoon suits me.\r\n",
        hours_ago(0)
    );
    deliver(store, "lunch-reply", &raw);
}

/// The inbox's rows read as `want`, top to bottom.
fn rows_read(window: &mut Window, what: &str, want: &[&str]) {
    window.until(what, |h| {
        let subjects: Vec<String> = (1..=h.count(".list .ds-list-item"))
            .filter_map(|n| h.text_of(&format!("{} .ds-thread-sub", super::look::row(n))))
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect();
        subjects == want
    });
}

#[test]
fn organise_by_conversation_turns_off_for_the_inbox_and_back() {
    let options = format!("[aria-label=\"{}\"]", copy::aria::MORE);
    let mut window = Window::open(answer);
    // Conversations: Alan's lunch is one row, under its first message's subject.
    let threaded = [
        LUNCH,
        "Flight to the conference",
        "The invoice for September",
        "Notes from the review",
    ];
    rows_read(&mut window, "the inbox lists conversations", &threaded);

    window.click(&options);
    window.until("the ⋯ offers it, checked", |h| {
        super::look::menu_item_of(h, copy::menus::ORGANISE_BY_CONVERSATION).is_some()
    });
    window.menu_item(copy::menus::ORGANISE_BY_CONVERSATION);
    rows_read(
        &mut window,
        "each message is a row, the reply beside what it answers",
        &[
            REPLY,
            "Flight to the conference",
            "The invoice for September",
            LUNCH,
            "Notes from the review",
        ],
    );
    let kept = load(&root_for(&window.dirs().config));
    assert_eq!(
        kept.window.single_messages,
        ["place:Inbox"],
        "kept for the inbox alone"
    );

    // A message opened from its row is shown alone, not with the reply in its conversation.
    window.open_subject(LUNCH);
    window.until("the reader shows the one message", |h| {
        h.count(".reader-body article.frame") == 1
    });

    window.click(&options);
    window.menu_item(copy::menus::ORGANISE_BY_CONVERSATION);
    rows_read(&mut window, "conversations again", &threaded);
    let kept = load(&root_for(&window.dirs().config));
    assert!(kept.window.single_messages.is_empty(), "nothing kept");
    window.until("the open conversation is shown whole", |h| {
        h.count(".reader-body article.frame") == 2
    });
}

#[test]
fn a_reply_answers_the_message_shown_alone_not_the_newest() {
    let options = format!("[aria-label=\"{}\"]", copy::aria::MORE);
    let mut window = Window::open(answer);
    window.click(&options);
    window.menu_item(copy::menus::ORGANISE_BY_CONVERSATION);
    rows_read(
        &mut window,
        "each message is a row",
        &[
            REPLY,
            "Flight to the conference",
            "The invoice for September",
            LUNCH,
            "Notes from the review",
        ],
    );
    // The invitation is older than the answer under it in the same conversation.
    window.open_subject(LUNCH);
    window.until("the reader shows the one message", |h| {
        h.count(".reader-body article.frame") == 1
    });
    window.press(&[], Key::Char('r'), 1);
    window.until("r opens a reply", |h| h.count(".inline-reply .c-body") == 1);
    let drafts = window.store.drafts(account()).expect("the drafts");
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let answered = drafts[0].in_reply_to.expect("the reply answers a message");
    assert_eq!(
        window.store.message(answered).expect("the message").subject,
        LUNCH,
        "the reply answered the newest message, not the one shown"
    );
}

#[test]
fn archiving_the_message_shown_alone_leaves_the_rest_of_its_conversation_in_the_inbox() {
    let options = format!("[aria-label=\"{}\"]", copy::aria::MORE);
    let mut window = Window::open(answer);
    window.click(&options);
    window.menu_item(copy::menus::ORGANISE_BY_CONVERSATION);
    window.open_subject(LUNCH);
    window.until("the reader shows the one message", |h| {
        h.count(".reader-body article.frame") == 1
    });
    window.press(&[], Key::Char('e'), 1);
    let of = |store: &mail_core::SqliteStore, subject: &str| {
        let query = mail_domain::Query {
            filter: mail_domain::Filter::All,
            sort: mail_domain::Sort {
                property: mail_domain::Property::Date,
                dir: mail_domain::SortDir::Desc,
            },
            page: mail_domain::PageReq {
                after: None,
                limit: 100,
            },
        };
        let threads = store.threads(&query, chrono::Utc::now()).unwrap().items;
        threads
            .iter()
            .filter_map(|summary| store.thread(summary.id).ok())
            .flat_map(|thread| thread.messages)
            .filter_map(|id| store.message(id).ok())
            .find(|message| message.subject == subject)
            .map(|message| message.mailbox)
    };
    window.until_stored("the invitation is archived", |store| {
        of(store, LUNCH) == Some(mail_domain::MailboxRole::Archive)
    });
    assert_eq!(
        of(&window.store, REPLY),
        Some(mail_domain::MailboxRole::Inbox),
        "the answer went with it"
    );
}
