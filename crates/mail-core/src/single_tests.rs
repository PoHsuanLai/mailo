use super::*;
use crate::support_tests::account;
use chrono::{TimeDelta, TimeZone};
use mail_domain::{
    Address, Body, Change, ChangeId, MailboxRole, MessageKey, Mute, Patch, Pin, ReadState, Sort,
    SortDir, Star, ThreadId,
};

/// Noon on 2026-09-28, and whole days from it.
fn day(n: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + TimeDelta::days(n)
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn id(n: u128) -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n))
}

/// Message `n` of conversation `t`, from `from`, `at`, filed as `role`, read or not.
fn message(
    n: u128,
    t: ThreadId,
    from: &str,
    at: DateTime<Utc>,
    role: MailboxRole,
    read: ReadState,
) -> Message {
    Message {
        id: id(n),
        thread: t,
        account: account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at,
        from: Address {
            name: None,
            email: from.to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("subject {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox: role,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

/// Conversation 1: Ada wrote to the inbox, I answered, Bob answered unread. Conversation 2: one
/// message, archived.
fn store(dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    let changes = [
        message(
            1,
            thread(1),
            "ada@example.test",
            day(-3),
            MailboxRole::Inbox,
            ReadState::Read,
        ),
        message(
            2,
            thread(1),
            "me@example.test",
            day(-2),
            MailboxRole::Sent,
            ReadState::Read,
        ),
        message(
            3,
            thread(1),
            "bob@example.test",
            day(-1),
            MailboxRole::Inbox,
            ReadState::Unread,
        ),
        message(
            4,
            thread(2),
            "cy@example.test",
            day(0),
            MailboxRole::Archive,
            ReadState::Read,
        ),
    ]
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
        .unwrap();
    store
}

fn query(filter: Filter, limit: u32) -> Query {
    Query {
        filter,
        sort: Sort {
            property: mail_domain::Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    }
}

fn listed_ids(page: &Page<Single>) -> Vec<MessageId> {
    page.items.iter().map(|single| single.message).collect()
}

/// A message on its own row says its own sender, subject and date, and carries the
/// conversation's id and the state only a conversation has.
#[test]
fn a_message_alone_is_its_own_row_under_its_conversation() {
    let ada = message(
        1,
        thread(1),
        "ada@example.test",
        day(-3),
        MailboxRole::Inbox,
        ReadState::Read,
    );
    let bob = message(
        3,
        thread(1),
        "bob@example.test",
        day(-1),
        MailboxRole::Archive,
        ReadState::Unread,
    );
    let conversation = ThreadSummary {
        pin: Pin::Rank(2),
        mute: Mute::Muted,
        ..ThreadSummary::derive(
            thread(1),
            &[ada.clone(), bob],
            mail_domain::Snooze::Inactive,
            Pin::Unpinned,
            Mute::Unmuted,
            mail_domain::FollowUp::Inactive,
        )
    };
    let row = alone(&ada, &conversation);
    assert_eq!(row.id, thread(1));
    assert_eq!(row.from.email, "ada@example.test");
    assert_eq!(row.subject, "subject 1");
    assert_eq!(row.last_date, day(-3));
    assert_eq!(row.message_count, 1);
    assert_eq!(
        row.read,
        ReadState::Read,
        "Bob's unread reply is not this row's"
    );
    assert!(row.mailboxes.contains(MailboxRole::Inbox));
    assert!(!row.mailboxes.contains(MailboxRole::Archive));
    assert_eq!((row.pin, row.mute), (Pin::Rank(2), Mute::Muted));
}

/// The place's filter is asked of each message alone: the inbox lists what is in the inbox and
/// not the reply sent from it, an unread search the unread message and not its read neighbours.
#[test]
fn a_place_lists_the_messages_its_filter_fits_on_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let now = day(1);
    let cases: [(&str, Filter, &[u128]); 4] = [
        (
            "inbox",
            mail_logic::mailbox_filter::for_role(MailboxRole::Inbox),
            &[3, 1],
        ),
        ("sent", Filter::InMailbox(MailboxRole::Sent), &[2]),
        ("unread", Filter::Read(ReadState::Unread), &[3]),
        ("everything", Filter::All, &[4, 3, 2, 1]),
    ];
    for (name, filter, want) in cases {
        let page = listed(&store, &query(filter, 50), now).unwrap();
        let want: Vec<MessageId> = want.iter().map(|n| id(*n)).collect();
        assert_eq!(listed_ids(&page), want, "{name}");
        assert_eq!(page.next, None, "{name}");
    }
}

/// A page holds `limit` messages the filter fits, however many it passed over, and the next
/// page starts after it.
#[test]
fn a_page_is_full_of_messages_that_fit_and_resumes_after_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut asked = query(mail_logic::mailbox_filter::for_role(MailboxRole::Inbox), 1);
    let first = listed(&store, &asked, day(1)).unwrap();
    assert_eq!(listed_ids(&first), [id(3)]);
    assert!(first.next.is_some(), "Ada's message is still to come");
    asked.page.after = first.next;
    let second = listed(&store, &asked, day(1)).unwrap();
    assert_eq!(
        listed_ids(&second),
        [id(1)],
        "my reply between them was passed over"
    );
    assert_eq!(second.next, None);
}

#[test]
fn a_folder_is_named_anywhere_in_a_filter() {
    let folder = Filter::InFolder(mail_domain::MailboxRef {
        account: account(),
        path: "Projects".to_owned(),
    });
    let cases = [
        ("alone", folder.clone(), true),
        (
            "under not, in an and",
            Filter::And(vec![
                Filter::Read(ReadState::Unread),
                Filter::Not(Box::new(folder)),
            ]),
            true,
        ),
        (
            "none",
            Filter::Or(vec![Filter::All, Filter::Starred(Star::Starred)]),
            false,
        ),
    ];
    for (name, filter, want) in cases {
        assert_eq!(names_folder(&filter), want, "{name}");
    }
}
