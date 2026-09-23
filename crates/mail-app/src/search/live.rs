//! The pipeline against the real SQLite index, where every other test uses a fake [`Source`].

use super::support::{at, sqlite_with};
use super::*;
use mail_domain::{Filter, PageReq, Property, Query, Sort, SortDir, TextMatch};
use mail_store::Store;

fn subjects(input: &str, store: &mail_store::SqliteStore) -> Vec<String> {
    let ranked = rank_query(
        input,
        store,
        &Affinity::default(),
        &Utc,
        &|_| Vec::new(),
        MENU,
        at(900),
    )
    .unwrap_or_else(|why| panic!("{input:?} is not a regex: {why}"));
    ranked.hits.into_iter().map(|(t, _)| t.subject).collect()
}

#[test]
fn half_a_word_finds_its_thread_through_the_index() {
    let (store, _dir) = sqlite_with(&[
        ("UIDVALIDITY changed overnight", "the server reset it", 100),
        ("Lunch on Friday", "sandwiches again", 200),
        ("校園郵件通知", "請查收", 300),
    ]);
    // Whole-word matching alone does not find it: that is what expansion adds.
    let whole = Query {
        filter: Filter::Text(TextMatch::Contains("uidval".to_owned())),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 10,
        },
    };
    let plain = store
        .threads(&whole, at(900))
        .map(|page| page.items.len())
        .unwrap_or(usize::MAX);
    assert_eq!(
        plain, 0,
        "the frozen whole-word filter must not match a prefix"
    );

    let cases: &[(&str, &[&str])] = &[
        ("uidval", &["UIDVALIDITY changed overnight"]),
        ("UIDVal", &["UIDVALIDITY changed overnight"]),
        ("lun", &["Lunch on Friday"]),
        ("郵", &["校園郵件通知"]),
        ("uidval ", &[]),
    ];
    for (input, want) in cases {
        assert_eq!(subjects(input, &store), *want, "typed {input:?}");
    }
}
