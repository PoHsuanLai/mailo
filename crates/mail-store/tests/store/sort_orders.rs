//! The list's every sort, either way, in both stores: the order a whole page has, and the same
//! order reached a page at a time across rows that sort alike.
//!
//! The parity proptest says the two stores agree and that paging repeats nothing; this says what
//! the order *is*, so a tie-break that changed in both stores at once still fails here.

use crate::support::*;
use mail_domain::*;
use mail_store::Store;

/// One conversation of one message: its date in seconds, subject, sender's name and address,
/// the size of the one part it carries, and whether it is unread and starred.
struct Row {
    n: u128,
    date: i64,
    subject: &'static str,
    name: Option<&'static str>,
    email: &'static str,
    size: u64,
    unread: bool,
    starred: bool,
}

/// Five conversations that tie on every key somewhere: t3 and t4 share a date, t1 and t3 a
/// subject once `Re:` is off, a sender once lowercased, and a size; t2 and t5 a sender and a
/// size.
const ROWS: [Row; 5] = [
    Row {
        n: 1,
        date: 100,
        subject: "Re: Banana",
        name: Some("Carol"),
        email: "c@example.test",
        size: 300,
        unread: true,
        starred: false,
    },
    Row {
        n: 2,
        date: 200,
        subject: "apple",
        name: None,
        email: "bob@example.test",
        size: 0,
        unread: false,
        starred: true,
    },
    Row {
        n: 3,
        date: 300,
        subject: "banana",
        name: Some("carol"),
        email: "carol@example.test",
        size: 300,
        unread: true,
        starred: true,
    },
    Row {
        n: 4,
        date: 300,
        subject: "Fwd: apple",
        name: Some("Alice"),
        email: "a@example.test",
        size: 50,
        unread: false,
        starred: false,
    },
    Row {
        n: 5,
        date: 400,
        subject: "cherry",
        name: None,
        email: "bob@example.test",
        size: 0,
        unread: true,
        starred: false,
    },
];

fn part(size: u64) -> Attachment {
    Attachment {
        name: "part.bin".to_owned(),
        mime: "application/octet-stream".to_owned(),
        size,
        content: PartContent::Remote {
            section: Section::root(),
        },
        inline: Inline::Attached,
    }
}

fn written(row: &Row) -> Message {
    Message {
        date: at(row.date),
        subject: row.subject.to_owned(),
        from: Address {
            name: row.name.map(str::to_owned),
            email: row.email.to_owned(),
        },
        read: if row.unread {
            ReadState::Unread
        } else {
            ReadState::Read
        },
        star: if row.starred {
            Star::Starred
        } else {
            Star::Unstarred
        },
        attachments: if row.size == 0 {
            vec![]
        } else {
            vec![part(row.size)]
        },
        ..message(row.n)
    }
}

fn holding(messages: impl IntoIterator<Item = Message>) -> Both {
    let b = Both::new();
    for message in messages {
        let upsert = patch(vec![Change::MessageUpsert(Box::new(message))]);
        b.sqlite.apply(acct_account(), &upsert).unwrap();
        b.memory.apply(acct_account(), &upsert).unwrap();
    }
    b
}

/// Which row a thread is, as `t1`..`t5`.
fn name(thread: ThreadId) -> String {
    let n = ROWS
        .iter()
        .find(|row| message(row.n).thread == thread)
        .map_or(0, |row| row.n);
    format!("t{n}")
}

/// Every thread under `sort`, `limit` at a time, following each page's cursor to the end.
fn paged(store: &dyn Store, sort: Sort, limit: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let query = Query {
            filter: Filter::All,
            sort,
            page: PageReq { after, limit },
        };
        let page = store.threads(&query, at(10_000)).unwrap();
        out.extend(page.items.iter().map(|s| name(s.id)));
        assert!(out.len() <= ROWS.len(), "{sort:?} by {limit} repeats rows");
        match page.next {
            Some(next) => after = Some(next),
            None => return out,
        }
    }
}

/// Each sort ascending; descending is the same order reversed, ties and all, since the date and
/// then the thread break every tie the same way round.
#[test]
fn every_sort_either_way_orders_and_pages_alike_in_both_stores() {
    let b = holding(ROWS.iter().map(written));
    let table: [(Property, [&str; 5]); 7] = [
        (Property::Date, ["t1", "t2", "t3", "t4", "t5"]),
        (Property::Subject, ["t2", "t4", "t1", "t3", "t5"]),
        (Property::From, ["t4", "t2", "t5", "t1", "t3"]),
        (Property::Sender, ["t4", "t2", "t5", "t1", "t3"]),
        (Property::Size, ["t2", "t5", "t4", "t1", "t3"]),
        (Property::Unread, ["t2", "t4", "t1", "t3", "t5"]),
        (Property::Starred, ["t1", "t4", "t5", "t2", "t3"]),
    ];
    for (property, ascending) in table {
        let descending: Vec<&str> = ascending.iter().rev().copied().collect();
        for (dir, want) in [
            (SortDir::Asc, ascending.to_vec()),
            (SortDir::Desc, descending),
        ] {
            let sort = Sort { property, dir };
            for (named, store) in b.each() {
                for limit in [1, 2, 3, 100] {
                    assert_eq!(
                        paged(store, sort, limit),
                        want,
                        "{named}: {property:?} {dir:?} by {limit}"
                    );
                }
            }
        }
    }
}

/// A conversation sorts by its newest message: its subject, its sender and its size, not the
/// first message's, which is what the row's subject shows.
#[test]
fn a_conversation_sorts_by_its_newest_message() {
    let thread = message(1).thread;
    let first = Message {
        subject: "zebra".to_owned(),
        attachments: vec![part(9_000)],
        ..written(&ROWS[0])
    };
    let newest = Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(11)),
        key: MessageKey::Rfc("m11@example.test".to_owned()),
        thread,
        date: at(150),
        subject: "Re: aardvark".to_owned(),
        from: Address {
            name: Some("Aaron".to_owned()),
            email: "aaron@example.test".to_owned(),
        },
        attachments: vec![],
        ..message(11)
    };
    let b = holding([first, newest, written(&ROWS[1])]);
    for (property, want) in [
        (Property::Subject, ["t1", "t2"]),
        (Property::Sender, ["t1", "t2"]),
        (Property::Size, ["t1", "t2"]),
    ] {
        let sort = Sort {
            property,
            dir: SortDir::Asc,
        };
        b.agree(|store| {
            assert_eq!(paged(store, sort, 1), want, "{property:?}");
        });
    }
}
