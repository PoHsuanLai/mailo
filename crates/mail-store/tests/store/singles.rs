//! The messages of the conversations a filter lists, one to a row (`messages_listed`): which are
//! read, in what order, and how a page resumes.

use crate::support::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// Message `n` in conversation `thread`, `n` seconds in, filed as `mailbox`.
fn in_thread(n: u128, thread: u128, mailbox: MailboxRole) -> Message {
    Message {
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(thread)),
        mailbox,
        ..message(n)
    }
}

fn put(store: &SqliteStore, messages: &[Message]) {
    let changes = messages
        .iter()
        .map(|m| Change::MessageUpsert(Box::new(m.clone())))
        .collect();
    store.apply(acct_account(), &patch(changes)).unwrap();
}

fn ids(page: &Page<mail_store::MessageRow>) -> Vec<MessageId> {
    page.items.iter().map(|row| row.message.id).collect()
}

fn id(n: u128) -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(n))
}

/// Conversation 1 is in the inbox and holds a reply sent from it; conversation 2 is archived.
fn store() -> SqliteStore {
    let store = sqlite();
    put(
        &store,
        &[
            in_thread(1, 1, MailboxRole::Inbox),
            in_thread(2, 1, MailboxRole::Sent),
            in_thread(3, 1, MailboxRole::Inbox),
            in_thread(4, 2, MailboxRole::Archive),
        ],
    );
    store
}

#[test]
fn every_message_of_a_listed_conversation_is_read_newest_first() {
    let store = store();
    let page = store
        .messages_listed(&query(Filter::InMailbox(MailboxRole::Inbox), 10), at(0))
        .unwrap();
    // The conversation is the inbox's, so all three of its messages are read, the reply sent
    // from it too: which of them the inbox shows is the caller's to decide. The archived
    // conversation is not listed, so its message is not read.
    assert_eq!(ids(&page), [id(3), id(2), id(1)]);
    assert_eq!(page.next, None);
    let conversation = store
        .thread(ThreadId::from_uuid(uuid::Uuid::from_u128(1)))
        .unwrap()
        .summary;
    assert!(
        page.items
            .iter()
            .all(|row| row.conversation == conversation),
        "each row carries its conversation as the store sums it up"
    );
}

#[test]
fn a_page_resumes_after_its_last_message() {
    let store = store();
    let mut asked = query(Filter::All, 2);
    let first = store.messages_listed(&asked, at(0)).unwrap();
    assert_eq!(ids(&first), [id(4), id(3)]);
    assert_eq!(first.next, first.items.last().map(|row| row.at.clone()));
    asked.page.after = first.next.clone();
    let second = store.messages_listed(&asked, at(0)).unwrap();
    assert_eq!(ids(&second), [id(2), id(1)]);
    assert_eq!(second.next, None);
    // Oldest first is the same rows the other way round.
    asked.page = PageReq {
        after: None,
        limit: 10,
    };
    asked.sort.dir = SortDir::Asc;
    let oldest = store.messages_listed(&asked, at(0)).unwrap();
    assert_eq!(ids(&oldest), [id(1), id(2), id(3), id(4)]);
}

#[test]
fn two_messages_sent_in_the_same_second_keep_their_places_across_pages() {
    let store = sqlite();
    let same = |n: u128| Message {
        date: at(5),
        ..in_thread(n, 7, MailboxRole::Inbox)
    };
    put(&store, &[same(11), same(12), same(13)]);
    let mut asked = query(Filter::All, 1);
    let mut seen = Vec::new();
    loop {
        let page = store.messages_listed(&asked, at(0)).unwrap();
        seen.extend(ids(&page));
        match page.next {
            Some(next) => asked.page.after = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, [id(13), id(12), id(11)], "the id breaks the tie");
}
