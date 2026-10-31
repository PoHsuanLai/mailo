//! The search language against the store: what the names a typed line stands for come from, and
//! the store listing what a typed `is:vip` asks for. Reading the line is `mail_logic::query`'s.

use crate::search::parse as split;
use crate::support_tests::{account as in_store, at, message};
use chrono::Utc;
use mail_domain::{
    Address, Change, ChangeId, Filter, PageReq, Patch, Property, Query, ReadState, Sort, SortDir,
    Star, TextMatch,
};
use mail_logic::query::{Known, parse_with};
use mail_store::{MemoryStore, Store};
use porter_core::AccountId;

fn account(n: u128) -> AccountId {
    mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(n))
}

fn accounts() -> Vec<(String, AccountId)> {
    vec![
        ("work@example.com".to_owned(), account(1)),
        ("me@home.example".to_owned(), account(2)),
        ("me@other.example".to_owned(), account(3)),
    ]
}

fn parse(input: &str) -> Filter {
    let accounts = accounts();
    let known = Known {
        labels: &[],
        accounts: &accounts,
    };
    parse_with(input, &Utc, &known)
}

fn from(word: &str) -> Filter {
    Filter::From(TextMatch::Contains(word.to_owned()))
}

/// The search box's own split keeps an either-or as one operator, not two words.
#[test]
fn the_search_box_keeps_an_either_or_whole() {
    let accounts = accounts();
    let known = Known {
        labels: &[],
        accounts: &accounts,
    };
    let parsed = split("from:ada OR from:bob lunch ", &Utc, &known);
    assert_eq!(
        parsed.operators,
        [Filter::Or(vec![from("ada"), from("bob")])]
    );
    assert_eq!(parsed.operator_tokens, ["from:ada OR from:bob"]);
    assert_eq!(parsed.words, ["lunch"]);
}

/// The other half: the store lists what `is:vip` asks for as the list is when it is asked,
/// by the address alone, and the same filter finds a different set once the list changes.
#[test]
fn the_store_lists_the_vips_conversations() {
    let store = MemoryStore::new();
    // (sender's address, the display name it is written with)
    let senders = [
        (1, "dana@example.test", None),
        (2, "bob@example.test", None),
        (3, "ada@example.test", None),
        (4, "notdana@example.test", None),
        // Anyone can write a VIP's address as their name.
        (5, "mallory@example.test", Some("dana@example.test")),
    ];
    for (n, email, name) in senders {
        let mut mail = message(
            n,
            &format!("from {email}"),
            "",
            n as i64,
            ReadState::Unread,
            Star::Unstarred,
        );
        mail.from = Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        };
        store
            .apply(
                in_store(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(mail))],
                },
            )
            .unwrap();
    }
    let listed = |typed: &str| -> Vec<String> {
        let query = Query {
            filter: parse(typed),
            sort: Sort {
                property: Property::Date,
                dir: SortDir::Desc,
            },
            page: PageReq {
                after: None,
                limit: 10,
            },
        };
        store
            .threads(&query, at(100))
            .unwrap()
            .items
            .into_iter()
            .map(|summary| summary.from.email)
            .collect()
    };
    assert_eq!(listed("is:vip"), Vec::<String>::new(), "nobody yet");
    store.put_vip("DANA@example.test", None).unwrap();
    store.put_vip("bob@example.test", None).unwrap();
    assert_eq!(
        listed("is:vip"),
        ["bob@example.test", "dana@example.test"],
        "the address alone, not a name that spells it"
    );
    assert_eq!(
        listed("-is:vip"),
        [
            "mallory@example.test",
            "notdana@example.test",
            "ada@example.test"
        ]
    );
    store.delete_vip("bob@example.test").unwrap();
    assert_eq!(
        listed("is:vip"),
        ["dana@example.test"],
        "the same filter follows the list"
    );
}
