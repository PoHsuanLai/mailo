//! The address book: what mail teaches it, what autocomplete offers from it, and the hand edits
//! and sync state kept beside it.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, as `tests/folders.rs` does for folders. The one SQLite-only test is the list header,
//! which needs the raw bytes the in-memory store does not keep.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{
    AddressBook, BookCard, Contact, Kind, MemoryStore, Origin, Settle, SqliteStore, Store,
};
use proptest::prelude::*;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ME: &str = "me@example.test";

fn at(day: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + day * 86_400, 0).unwrap()
}

fn sqlite() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, '{}', datetime('now'))",
            [ACCOUNT.to_string(), ME.to_owned()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', ?3, '\"default\"')",
            [IDENTITY.to_string(), ACCOUNT.to_string(), ME.to_owned()],
        )
        .unwrap();
    }
    (store, dir)
}

/// Run `scenario` on both stores and hand back what each produced.
fn both<T>(scenario: impl Fn(&dyn Store) -> T) -> (T, T) {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    (scenario(&sqlite), scenario(&memory))
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// A message `n`, `day` days in, from `from` to `to`, in `mailbox`.
fn message(n: u128, day: i64, mailbox: MailboxRole, from: Address, to: Vec<Address>) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1_000_000)),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(day),
        from,
        reply_to: vec![],
        to,
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn received(n: u128, day: i64, from: Address) -> Message {
    message(n, day, MailboxRole::Inbox, from, vec![addr(None, ME)])
}

fn sent(n: u128, day: i64, to: Vec<Address>) -> Message {
    message(n, day, MailboxRole::Sent, addr(Some("Me"), ME), to)
}

fn deliver_at(store: &dyn Store, m: &Message, path: &str, uid: u32, raw: BlobId) {
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: path.to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: path.to_owned(),
                        uidvalidity: 7,
                        uid,
                    },
                    key: m.key.clone(),
                    raw,
                    message: m.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
}

fn deliver(store: &dyn Store, m: &Message) {
    let path = if m.mailbox == MailboxRole::Sent {
        "Sent"
    } else {
        "INBOX"
    };
    let uid = (m.id.as_uuid().as_u128() % 1_000_000) as u32;
    deliver_at(store, m, path, uid, BlobId::generate());
}

fn offered(store: &dyn Store, typed: &str) -> Vec<String> {
    store
        .contacts_matching(typed, 10)
        .unwrap()
        .into_iter()
        .map(|c| c.address)
        .collect()
}

#[test]
fn received_and_sent_mail_teach_both_stores_the_same_book() {
    let (a, b) = both(|store| {
        deliver(
            store,
            &received(1, 0, addr(Some("Ada Lovelace"), "Ada@Example.test")),
        );
        deliver(
            store,
            &received(2, 3, addr(Some("Ada L."), "ada@example.test")),
        );
        deliver(
            store,
            &sent(3, 5, vec![addr(Some("Bob"), "bob@example.test")]),
        );
        store.contacts().unwrap()
    });
    assert_eq!(a, b);
    let ada = a.iter().find(|c| c.address == "ada@example.test").unwrap();
    assert_eq!(ada.name.as_deref(), Some("Ada L."), "the most recent name");
    assert_eq!(ada.received.count, 2);
    assert_eq!(ada.received.last, Some(at(3)));
    assert_eq!(ada.written.count, 0);
    assert_eq!(ada.account, Some(ACCOUNT));
    let bob = a.iter().find(|c| c.address == "bob@example.test").unwrap();
    assert_eq!(bob.written.count, 1);
    let me = a.iter().find(|c| c.address == ME).unwrap();
    assert_eq!(me.kind, Kind::Own);
}

#[test]
fn a_message_seen_again_is_not_counted_again() {
    let (a, b) = both(|store| {
        let m = received(1, 0, addr(None, "ada@example.test"));
        deliver_at(store, &m, "INBOX", 1, BlobId::generate());
        // Fetched again, as a resurvey after a restart does.
        deliver_at(store, &m, "INBOX", 1, BlobId::generate());
        // The same message under a second mailbox, as Gmail's All Mail shows it.
        deliver_at(store, &m, "Archive", 9, BlobId::generate());
        store.contact("ADA@example.test").unwrap().unwrap()
    });
    assert_eq!(a, b);
    assert_eq!(a.received.count, 1);
}

#[test]
fn spam_and_drafts_teach_nothing() {
    let (a, b) = both(|store| {
        deliver(
            store,
            &message(
                1,
                0,
                MailboxRole::Spam,
                addr(None, "prince@example.test"),
                vec![],
            ),
        );
        deliver(
            store,
            &message(
                2,
                0,
                MailboxRole::Drafts,
                addr(None, ME),
                vec![addr(None, "halfway@example.test")],
            ),
        );
        store.contacts().unwrap()
    });
    assert_eq!(a, b);
    assert_eq!(a, vec![]);
}

#[test]
fn someone_written_to_once_ranks_above_a_frequent_sender() {
    let (a, b) = both(|store| {
        for n in 0..5 {
            deliver(
                store,
                &received(n + 1, n as i64, addr(Some("Alerts"), "alerts@example.test")),
            );
        }
        deliver(
            store,
            &sent(10, 0, vec![addr(Some("Alice"), "alice@example.test")]),
        );
        offered(store, "al")
    });
    assert_eq!(a, b);
    assert_eq!(a, ["alice@example.test", "alerts@example.test"]);
}

#[test]
fn no_reply_senders_are_offered_only_once_written_to() {
    let (a, b) = both(|store| {
        deliver(
            store,
            &received(1, 0, addr(Some("Shop"), "no-reply@shop.example.test")),
        );
        deliver(
            store,
            &received(2, 0, addr(Some("Shop Help"), "help@shop.example.test")),
        );
        let before = offered(store, "shop");
        deliver(
            store,
            &sent(3, 1, vec![addr(None, "no-reply@shop.example.test")]),
        );
        (before, offered(store, "shop"))
    });
    assert_eq!(a, b);
    assert_eq!(a.0, ["help@shop.example.test"]);
    assert_eq!(
        a.1,
        ["no-reply@shop.example.test", "help@shop.example.test"]
    );
}

#[test]
fn the_users_own_address_is_never_offered() {
    let (a, b) = both(|store| {
        deliver(store, &sent(1, 0, vec![addr(None, "bob@example.test")]));
        // A copy to themselves, arriving in the inbox.
        deliver(store, &received(2, 1, addr(Some("Me"), ME)));
        offered(store, "")
    });
    assert_eq!(a, b);
    assert_eq!(a, ["bob@example.test"]);
}

#[test]
fn a_prefix_finds_any_word_of_the_name_or_the_address_in_both_stores() {
    const CASES: &[(&str, &[&str])] = &[
        ("ren", &["renee@example.test"]),
        ("RENÉE", &["renee@example.test"]),
        ("müller", &["renee@example.test"]),
        ("mul", &["renee@example.test"]),
        ("ren mul", &["renee@example.test"]),
        ("j.sm", &["j.smith@work.example.test"]),
        ("smith", &["j.smith@work.example.test"]),
        ("work.ex", &["j.smith@work.example.test"]),
        ("exam", &["renee@example.test"]),
        ("zzz", &[]),
    ];
    let (a, b) = both(|store| {
        deliver(
            store,
            &received(1, 0, addr(Some("Renée Müller"), "renee@example.test")),
        );
        deliver(
            store,
            &received(2, 0, addr(Some("Jo"), "j.smith@work.example.test")),
        );
        CASES
            .iter()
            .map(|(typed, _)| offered(store, typed))
            .collect::<Vec<_>>()
    });
    assert_eq!(a, b);
    for ((typed, expected), got) in CASES.iter().zip(&a) {
        assert_eq!(got, expected, "{typed:?}");
    }
}

#[test]
fn autocomplete_stops_at_k() {
    let (a, b) = both(|store| {
        for n in 0..8u128 {
            deliver(
                store,
                &received(n + 1, n as i64, addr(None, &format!("p{n}@example.test"))),
            );
        }
        store
            .contacts_matching("p", 3)
            .unwrap()
            .into_iter()
            .map(|c| c.address)
            .collect::<Vec<_>>()
    });
    assert_eq!(a, b);
    assert_eq!(a, ["p7@example.test", "p6@example.test", "p5@example.test"]);
}

#[test]
fn a_name_given_by_hand_survives_later_mail_and_can_be_given_back() {
    let (a, b) = both(|store| {
        deliver(
            store,
            &received(1, 0, addr(Some("A. Lovelace"), "ada@example.test")),
        );
        let given = store
            .put_contact("Ada@example.test", Some("Ada"), &Origin::Manual)
            .unwrap();
        deliver(
            store,
            &received(2, 1, addr(Some("Countess"), "ada@example.test")),
        );
        let kept = store.contact("ada@example.test").unwrap().unwrap();
        store
            .put_contact("ada@example.test", None, &Origin::History)
            .unwrap();
        deliver(
            store,
            &received(3, 2, addr(Some("Ada King"), "ada@example.test")),
        );
        let returned = store.contact("ada@example.test").unwrap().unwrap();
        (given, kept, returned)
    });
    assert_eq!(a, b);
    let (given, kept, returned) = a;
    assert_eq!(given.name.as_deref(), Some("Ada"));
    assert_eq!(given.received.count, 1, "the counts are kept");
    assert_eq!(kept.name.as_deref(), Some("Ada"));
    assert_eq!(kept.received.count, 2);
    assert_eq!(returned.name.as_deref(), Some("Ada King"));
}

#[test]
fn a_contact_added_by_hand_is_offered_and_can_be_removed() {
    let (a, b) = both(|store| {
        let added = store
            .put_contact(" Zed@Example.test ", Some("Zed"), &Origin::Manual)
            .unwrap();
        let found = offered(store, "zed");
        let removed = store.delete_contact("zed@example.test").unwrap();
        let again = store.delete_contact("zed@example.test").unwrap();
        (added, found, removed, again, store.contacts().unwrap())
    });
    assert_eq!(a, b);
    let (added, found, removed, again, left) = a;
    assert_eq!(added.address, "zed@example.test");
    assert_eq!(added.origin, Origin::Manual);
    assert_eq!(found, ["zed@example.test"]);
    assert!(removed);
    assert!(!again);
    assert_eq!(left, vec![]);
}

#[test]
fn something_that_is_not_an_address_is_refused() {
    let (a, b) = both(|store| {
        (
            store
                .put_contact("not an address", Some("X"), &Origin::Manual)
                .is_err(),
            store.contact("nobody").unwrap(),
        )
    });
    assert_eq!(a, b);
    assert_eq!(a, (true, None));
}

/// Queue and confirm a submission of `draft`, as the outbox does when SMTP accepts it.
fn send(store: &dyn Store, draft: &Draft, now: DateTime<Utc>) {
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let rcpt_to = draft
        .to
        .iter()
        .chain(&draft.cc)
        .chain(&draft.bcc)
        .map(|a| a.email.clone())
        .collect();
    let id = store
        .enqueue(
            ACCOUNT,
            RemoteIntent::Send {
                draft: draft.id,
                raw: BlobId::generate(),
                mail_from: ME.to_owned(),
                rcpt_to,
            },
            &Patch {
                id: ChangeId::generate(),
                changes: vec![],
            },
            now,
        )
        .unwrap()
        .unwrap();
    store.outbox_settle(id, Settle::Ok, now).unwrap();
}

fn draft(to: Vec<Address>, subject: &str) -> Draft {
    Draft {
        id: DraftId::from_uuid(uuid::Uuid::from_u128(77)),
        account: ACCOUNT,
        identity: IDENTITY,
        to,
        cc: vec![],
        bcc: vec![addr(None, "hidden@example.test")],
        subject: subject.to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: String::new(),
        html: None,
        attachments: vec![],
        state: SendState::Queued,
        updated: at(0),
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
    }
}

#[test]
fn a_confirmed_send_counts_its_recipients_once_even_when_its_sent_copy_arrives() {
    let (a, b) = both(|store| {
        let to = vec![addr(Some("Bob Typed"), "Bob@example.test")];
        send(store, &draft(to.clone(), "Plans"), at(4));
        let after_send = store.contact("bob@example.test").unwrap().unwrap();
        // The Sent copy of that message, synced later: recognised, not counted again.
        let mut copy = sent(5, 4, to.clone());
        copy.subject = "Plans".to_owned();
        deliver(store, &copy);
        let after_copy = store.contact("bob@example.test").unwrap().unwrap();
        // A second, different message to Bob does count.
        deliver(store, &sent(6, 6, to));
        let after_next = store.contact("bob@example.test").unwrap().unwrap();
        (
            after_send,
            after_copy,
            after_next,
            store.contact("hidden@example.test").unwrap(),
        )
    });
    assert_eq!(a, b);
    let (after_send, after_copy, after_next, hidden) = a;
    assert_eq!(after_send.written.count, 1);
    assert_eq!(after_send.name.as_deref(), Some("Bob Typed"));
    assert_eq!(after_copy.written.count, 1);
    assert_eq!(after_next.written.count, 2);
    assert_eq!(
        hidden.map(|c| c.written.count),
        Some(1),
        "Bcc is written to"
    );
}

#[test]
fn an_address_books_sync_state_round_trips() {
    let book = AddressBook {
        url: "https://dav.example.test/book/".to_owned(),
        token: Some("sync-1".to_owned()),
        cards: [(
            "/book/a.vcf".to_owned(),
            BookCard {
                etag: "\"1\"".to_owned(),
                addresses: vec!["a@example.test".to_owned()],
                vcard: "BEGIN:VCARD\r\nEND:VCARD\r\n".to_owned(),
            },
        )]
        .into(),
        account: Some(ACCOUNT),
        login: Some("me".to_owned()),
    };
    let (a, b) = both(|store| {
        let before = store.address_book(&book.url).unwrap();
        store.put_address_book(&book).unwrap();
        let mut next = book.clone();
        next.token = Some("sync-2".to_owned());
        store.put_address_book(&next).unwrap();
        let mut other = book.clone();
        other.url = "https://dav.example.test/another/".to_owned();
        store.put_address_book(&other).unwrap();
        (
            before,
            store.address_book(&book.url).unwrap(),
            store
                .address_books()
                .unwrap()
                .into_iter()
                .map(|b| b.url)
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(a, b);
    assert_eq!(a.0, None);
    assert_eq!(a.1.unwrap().token.as_deref(), Some("sync-2"));
    assert_eq!(
        a.2,
        [
            "https://dav.example.test/another/",
            "https://dav.example.test/book/"
        ]
    );
}

#[test]
fn a_sender_whose_mail_came_through_a_list_is_not_offered_until_written_to() {
    // SQLite only: the header is in the raw bytes, which the in-memory store does not keep.
    let (store, _dir) = sqlite();
    let raw = store
        .blobs()
        .put(
            &store.connection(),
            b"From: Poster <poster@example.test>\r\nList-Id: Talk <talk.example.test>\r\n\r\nhi",
        )
        .unwrap();
    let m = received(1, 0, addr(Some("Poster"), "poster@example.test"));
    deliver_at(&store, &m, "INBOX", 1, raw);
    let poster = store.contact("poster@example.test").unwrap().unwrap();
    assert_eq!(poster.kind, Kind::Bulk);
    assert!(offered(&store, "poster").is_empty());
    deliver(&store, &sent(2, 1, vec![addr(None, "poster@example.test")]));
    assert_eq!(offered(&store, "poster"), ["poster@example.test"]);
}

// ------------------------------------------------------------------ parity, generated

const PEOPLE: &[(&str, &str)] = &[
    ("Ada Lovelace", "ada@example.test"),
    ("Bob Ng", "bob@example.test"),
    ("Chloé Durand", "chloe.durand@example.test"),
    ("", "noreply@shop.example.test"),
    ("Dmitri", "d@mail.example.test"),
];

fn event() -> impl Strategy<Value = (usize, usize, u8, i64)> {
    // (who, which name form, what kind of message, day)
    (0..PEOPLE.len(), 0..3usize, 0..4u8, 0..400i64)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn both_stores_learn_and_rank_the_same_book(events in prop::collection::vec(event(), 1..30)) {
        let prefixes = ["", "a", "b", "ch", "chlo", "dur", "d", "shop", "example", "mail.ex", "x"];
        let (a, b) = both(|store| {
            for (n, (who, form, kind, day)) in events.iter().enumerate() {
                let (name, email) = PEOPLE[*who];
                let name = match form {
                    0 => None,
                    1 => Some(name.to_owned()),
                    _ => Some(format!("{name} (work)")),
                };
                let them = Address { name, email: email.to_owned() };
                let n = n as u128 + 1;
                let m = match kind {
                    0 | 1 => received(n, *day, them),
                    2 => sent(n, *day, vec![them]),
                    _ => message(n, *day, MailboxRole::Archive, them, vec![]),
                };
                deliver(store, &m);
            }
            let book: Vec<Contact> = store.contacts().unwrap();
            let offered: Vec<Vec<String>> = prefixes.iter().map(|p| offered(store, p)).collect();
            (book, offered)
        });
        prop_assert_eq!(a, b);
    }
}
