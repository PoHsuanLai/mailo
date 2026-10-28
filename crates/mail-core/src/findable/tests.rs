//! Findable things read from a scratch store: which conversations are kept, how many, and how each
//! is worded.

use super::*;
use crate::support_tests::account;
use chrono::TimeZone;
use mail_domain::{
    Body, Change, ChangeId, Message, MessageId, MessageKey, Patch, ReadState, Star, ThreadId,
};
use mail_store::Origin;

fn at(minute: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + minute * 60, 0)
        .single()
        .expect("a time")
}

fn person(name: &str, email: &str) -> Address {
    Address {
        name: (!name.is_empty()).then(|| name.to_owned()),
        email: email.to_owned(),
    }
}

/// Message `n` of conversation `thread`, from `from` to the person, in `mailbox`.
fn message(n: u128, thread: u128, subject: &str, from: Address, mailbox: MailboxRole) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + thread)),
        account: account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from,
        reply_to: vec![],
        to: vec![person("", "me@example.test")],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Unread,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn store_of(messages: Vec<Message>) -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SqliteStore::in_memory(dir.path()).expect("a store");
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    let changes = messages
        .into_iter()
        .map(|m| Change::MessageUpsert(Box::new(m)))
        .collect();
    store
        .apply(
            account(),
            &Patch {
                id: ChangeId::generate(),
                changes,
            },
        )
        .expect("the messages");
    (store, dir)
}

fn ada() -> Address {
    person("Ada Lovelace", "ada@example.test")
}

fn titles(found: &[Findable]) -> Vec<&str> {
    found.iter().map(|f| f.title.as_str()).collect()
}

#[test]
fn a_conversation_only_in_trash_or_spam_is_left_out() {
    let (store, _dir) = store_of(vec![
        message(1, 1, "In the inbox", ada(), MailboxRole::Inbox),
        message(2, 2, "Only in the trash", ada(), MailboxRole::Trash),
        message(3, 3, "Only in spam", ada(), MailboxRole::Spam),
        message(4, 4, "Trash and spam", ada(), MailboxRole::Trash),
        message(5, 4, "Trash and spam", ada(), MailboxRole::Spam),
        message(6, 5, "Trash and the inbox", ada(), MailboxRole::Trash),
        message(7, 5, "Trash and the inbox", ada(), MailboxRole::Inbox),
        message(8, 6, "Archived", ada(), MailboxRole::Archive),
    ]);
    let own = own_addresses(&store).expect("own");
    let found = conversations(&store, &own, CONVERSATIONS, at(100)).expect("read");
    assert_eq!(
        titles(&found),
        ["Archived", "Trash and the inbox", "In the inbox"]
    );
}

#[test]
fn the_bound_keeps_the_newest_across_pages() {
    let all = (1..=1_203)
        .map(|n| message(n, n, &format!("Note {n}"), ada(), MailboxRole::Inbox))
        .collect();
    let (store, _dir) = store_of(all);
    let own = own_addresses(&store).expect("own");
    let found = conversations(&store, &own, 1_001, at(2_000)).expect("read");
    assert_eq!(found.len(), 1_001);
    assert_eq!(found[0].title, "Note 1203");
    assert_eq!(found[1_000].title, "Note 203");
    let few = conversations(&store, &own, 3, at(2_000)).expect("read");
    assert_eq!(titles(&few), ["Note 1203", "Note 1202", "Note 1201"]);
}

#[test]
fn a_conversation_is_its_subject_its_other_people_and_their_words() {
    let mut first = message(1, 1, "", ada(), MailboxRole::Inbox);
    first.cc = vec![person("Grace Hopper", "grace@example.test")];
    let reply = message(
        2,
        1,
        "Re:",
        person("", "linus@example.test"),
        MailboxRole::Inbox,
    );
    let (store, _dir) = store_of(vec![first, reply]);
    let found = read(&store, at(100)).expect("read");
    let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
    let conversation = &found[&(Kind::Conversation, thread.to_string())];
    // The subject of the oldest message, which had none.
    assert_eq!(conversation.title, mail_domain::NO_SUBJECT);
    assert_eq!(
        conversation.subtitle,
        "linus@example.test, Ada Lovelace, Grace Hopper"
    );
    assert_eq!(
        conversation.keywords,
        [
            "linus@example.test",
            "ada@example.test",
            "Ada Lovelace",
            "grace@example.test",
            "Grace Hopper"
        ]
    );
    assert_eq!(conversation.updated, at(2));
    assert!(
        !conversation
            .keywords
            .iter()
            .any(|word| word.contains("me@example.test")),
        "the person's own address finds every conversation, so it finds none"
    );
}

#[test]
fn a_contact_is_its_name_or_address_and_the_book_adds_its_other_words() {
    let (store, _dir) = store_of(Vec::new());
    store
        .put_contact("ada@example.test", Some("Ada Lovelace"), &Origin::Manual)
        .expect("ada");
    store
        .put_contact("grace@example.test", None, &Origin::Manual)
        .expect("grace");
    let url = "https://dav.example.test/book/";
    let mut book = mail_store::AddressBook {
        url: url.to_owned(),
        ..Default::default()
    };
    book.cards.insert(
        format!("{url}ada.vcf"),
        mail_store::BookCard {
            uid: None,
            etag: "1".to_owned(),
            addresses: vec![
                "ada@example.test".to_owned(),
                "ada@work.example.test".to_owned(),
            ],
            vcard: "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada Lovelace\r\nEMAIL:ada@example.test\r\n\
                    EMAIL:ada@work.example.test\r\nORG:Analytical Engines\r\nEND:VCARD\r\n"
                .to_owned(),
        },
    );
    store.put_address_book(&book).expect("the book");
    let found = contacts(&store, CONTACTS).expect("read");
    let ada = found
        .iter()
        .find(|c| c.key == "ada@example.test")
        .expect("ada");
    assert_eq!(
        (ada.title.as_str(), ada.subtitle.as_str()),
        ("Ada Lovelace", "ada@example.test")
    );
    assert_eq!(
        ada.keywords,
        ["ada@work.example.test", "Analytical Engines"]
    );
    let grace = found
        .iter()
        .find(|c| c.key == "grace@example.test")
        .expect("grace");
    assert_eq!(grace.title, "grace@example.test");
    assert!(grace.keywords.is_empty());
    assert_eq!(contacts(&store, 1).expect("read").len(), 1);
}

#[test]
fn changes_are_what_is_new_or_different_and_what_is_gone() {
    let (store, _dir) = store_of(vec![
        message(1, 1, "Lunch", ada(), MailboxRole::Inbox),
        message(2, 2, "Report", ada(), MailboxRole::Inbox),
    ]);
    let before = read(&store, at(100)).expect("read");
    assert_eq!(changes(&before, &before), Changes::default());
    let mut after = before.clone();
    let lunch = (
        Kind::Conversation,
        ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001)).to_string(),
    );
    let report = (
        Kind::Conversation,
        ThreadId::from_uuid(uuid::Uuid::from_u128(0x7002)).to_string(),
    );
    after.remove(&lunch);
    after.get_mut(&report).expect("report").title = "Report, again".to_owned();
    let moved = changes(&before, &after);
    assert_eq!(titles(&moved.upserts), ["Report, again"]);
    assert_eq!(moved.removals, [lunch]);
}
