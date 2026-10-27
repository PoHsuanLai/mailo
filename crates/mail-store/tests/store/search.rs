//! Prefix terms and the top hits of a listed window.
//!
//! Folding, the range (so `%` and `_` are literals), CJK bigrams, rank order, limits, and the
//! fact that suggestions are not per account. Order is not compared across the two stores; the
//! parity test owns the set, and one case below is where the two orders diverge.

use crate::support::*;
use mail_domain::*;
use mail_store::testing::seed_account;
use mail_store::{MemoryStore, SqliteStore, Store, Term};
use porter_core::AccountId;

struct Mail<'a> {
    account: AccountId,
    subject: &'a str,
    body: Option<&'a str>,
    secs: i64,
}

/// A message to the first account with only a subject, `secs` in.
fn mail(subject: &str, secs: i64) -> Mail<'_> {
    Mail {
        account: acct_account(),
        subject,
        body: None,
        secs,
    }
}

struct Held {
    sqlite: SqliteStore,
    memory: MemoryStore,
    threads: Vec<ThreadId>,
}

/// Each of `mails` in a thread of its own, in both stores.
fn load(mails: &[Mail<'_>]) -> Held {
    let sqlite = sqlite();
    let memory = MemoryStore::new();
    let raw = sqlite.blobs().put(b"raw").unwrap();
    let mut threads = Vec::new();
    for (i, mail) in mails.iter().enumerate() {
        let message = Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: mail.account.clone(),
            date: at(mail.secs),
            // A sender whose words no prefix below asks for.
            from: Address {
                name: None,
                email: "q@q.qq".into(),
            },
            subject: mail.subject.into(),
            body: Body::Present {
                text: mail.body.map(str::to_owned),
                raw,
            },
            ..message(i as u128)
        };
        threads.push(message.thread);
        let upsert = patch(vec![Change::MessageUpsert(Box::new(message))]);
        sqlite.apply(mail.account.clone(), &upsert).unwrap();
        memory.apply(mail.account.clone(), &upsert).unwrap();
    }
    Held {
        sqlite,
        memory,
        threads,
    }
}

fn texts(store: &dyn Store, prefix: &str) -> Vec<String> {
    store
        .terms_with_prefix(prefix, 32)
        .unwrap()
        .into_iter()
        .map(|term| term.text)
        .collect()
}

fn stores(held: &Held) -> [&dyn Store; 2] {
    [&held.sqlite, &held.memory]
}

/// The newest `limit` threads `filter` lists: the window a caller hands `top_hits`, taken the
/// way callers take it.
fn listed(store: &dyn Store, filter: &Filter, limit: u32) -> Vec<ThreadId> {
    let query = query(filter.clone(), limit);
    store
        .threads(&query, at(0))
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect()
}

/// `top_hits` over the newest 50 threads `filter` lists.
fn ranked(store: &dyn Store, filter: &Filter, k: usize) -> Vec<(ThreadSummary, f64)> {
    let window = listed(store, filter, 50);
    store.top_hits(filter, k, &window, at(0)).unwrap()
}

/// `Rés` matches the indexed `resume`, case is ignored, and an empty prefix is not the vocabulary.
#[test]
fn folded_prefixes_match_indexed_terms() {
    const CASES: &[(&str, &[&str])] = &[
        ("Rés", &["resume"]),
        ("résumé", &["resume"]),
        ("RES", &["resume"]),
        ("res", &["resume"]),
        ("", &[]),
    ];
    let held = load(&[mail("Résumé planned", 1)]);
    for store in stores(&held) {
        for (prefix, expect) in CASES {
            assert_eq!(texts(store, prefix), *expect, "prefix {prefix:?}");
        }
    }
}

/// Prefix `zz` against the documents `zz`, `zza` and `zz{`. `%` and `_` are literals: a `LIKE`
/// pattern would return the whole vocabulary for `%` and every term for `_`.
#[test]
fn range_prefix_keeps_percent_and_underscore_literal() {
    const CASES: &[(&str, &[&str])] = &[
        ("zz", &["zz", "zza"]),
        ("zz{", &["zz", "zza"]),
        ("%", &[]),
        ("_", &[]),
        ("100%", &["100"]),
    ];
    let held = load(&[
        mail("zz", 1),
        mail("zza", 2),
        mail("zz{", 3),
        mail("z{", 4),
        mail("alpha", 5),
        mail("100%", 6),
    ]);
    for store in stores(&held) {
        for (prefix, expect) in CASES {
            let got = texts(store, prefix);
            assert_eq!(got, *expect, "prefix {prefix:?} matched {got:?}");
        }
        let zz = store.terms_with_prefix("zz", 32).unwrap();
        assert_eq!(
            zz[0],
            Term {
                text: "zz".into(),
                docs: 2,
                cjk_bigram: false
            }
        );
        assert_eq!(
            zz[1],
            Term {
                text: "zza".into(),
                docs: 1,
                cjk_bigram: false
            }
        );
    }
}

/// One ideograph matches the bigrams that start with it, and each of those is flagged.
#[test]
fn one_ideograph_returns_flagged_bigrams() {
    const CASES: &[(&str, &str)] = &[("電", "電子"), ("子", "子郵"), ("郵", "郵件")];
    let held = load(&[mail("電子郵件", 1)]);
    for store in stores(&held) {
        for (prefix, term) in CASES {
            let got = store.terms_with_prefix(prefix, 32).unwrap();
            assert!(
                got.iter().all(|t| t.cjk_bigram),
                "prefix {prefix:?} returned an unflagged term: {got:?}"
            );
            assert_eq!(
                got.into_iter().map(|t| t.text).collect::<Vec<_>>(),
                vec![(*term).to_owned()],
                "prefix {prefix:?}"
            );
        }
    }

    // A run of one ideograph is stored as that character, and it is not a bigram.
    let lone = load(&[mail("電", 1)]);
    for store in stores(&lone) {
        let got = store.terms_with_prefix("電", 32).unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].text, "電");
        assert!(!got[0].cjk_bigram);
    }
}

/// Frequency, then the term text. `ac` is in two messages; `aa` sorts before `ab`.
#[test]
fn terms_sort_by_frequency_then_text() {
    let held = load(&[mail("ac", 1), mail("ac", 2), mail("aa", 3), mail("ab", 4)]);
    let expect = ["ac", "aa", "ab"];
    for store in stores(&held) {
        let got = store.terms_with_prefix("a", 32).unwrap();
        assert_eq!(
            got.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            expect
        );
        assert_eq!(got[0].docs, 2);
        assert_eq!(got[1].docs, 1);
        assert_eq!(got[2].docs, 1);
    }
}

/// Order is not compared across the two stores.
///
/// SQLite orders by negated bm25, so the older thread (the word twice in a short subject)
/// outranks one appearance in a long body and comes first: the order, not the scores, and a
/// date ordering would come out the other way. The memory store scores everything 0.0 and orders by `last_date`, so the newer
/// thread comes first. Asserting those two sequences equal would fail on this corpus, and it
/// would be testing a difference the stores are supposed to have.
#[test]
fn order_is_not_compared_across_the_two_stores() {
    let long = format!("{}kite", "padding ".repeat(400));
    let held = load(&[
        Mail {
            account: acct_account(),
            subject: "kite kite",
            body: Some("ok"),
            secs: 1,
        },
        Mail {
            account: acct_account(),
            subject: "hello",
            body: Some(&long),
            secs: 2,
        },
    ]);
    let filter = Filter::Text(TextMatch::Contains("kite".into()));
    let sql = ranked(&held.sqlite, &filter, 10);
    let mem = ranked(&held.memory, &filter, 10);
    let sql_ids: Vec<_> = sql.iter().map(|(s, _)| s.id).collect::<Vec<_>>();
    let mem_ids: Vec<_> = mem.iter().map(|(s, _)| s.id).collect::<Vec<_>>();
    assert_eq!(
        sql_ids.iter().collect::<std::collections::BTreeSet<_>>(),
        mem_ids.iter().collect::<std::collections::BTreeSet<_>>()
    );
    assert!(mem.iter().all(|(_, score)| *score == 0.0));
    assert_eq!(mem_ids[0], held.threads[1], "memory keeps newest first");
    assert_eq!(
        sql_ids[0], held.threads[0],
        "sqlite keeps the better match first"
    );
    assert_ne!(sql_ids, mem_ids);
}

/// A term that exists only on the other account is still suggested. Filtering to the first
/// account then drops the other's thread, which is why suggesting it costs nothing.
#[test]
fn suggestions_come_from_every_account() {
    let held = load(&[
        mail("apple harvest", 1),
        Mail {
            account: acct_other(),
            subject: "xylophone solo",
            body: None,
            secs: 2,
        },
    ]);
    let filter_a = Filter::And(vec![
        Filter::Account(acct_account()),
        Filter::Text(TextMatch::Contains("xylophone".into())),
    ]);
    let filter_b = Filter::And(vec![
        Filter::Account(acct_other()),
        Filter::Text(TextMatch::Contains("xylophone".into())),
    ]);
    for store in stores(&held) {
        let suggested = texts(store, "xylo");
        assert_eq!(suggested, vec!["xylophone".to_owned()]);
        assert!(ranked(store, &filter_a, 10).is_empty());
        let found = ranked(store, &filter_b, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.account, acct_other());
    }
}

/// Readers are separate connections. The vocab table is created on each of them.
#[test]
fn terms_are_visible_from_a_read_connection() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap();
    seed_account(&sqlite, acct_account(), "a@example.test");
    let raw = sqlite.blobs().put(b"raw").unwrap();
    let message = Message {
        subject: "Résumé".into(),
        body: Body::Present { text: None, raw },
        ..message(1)
    };
    sqlite
        .apply(
            acct_account(),
            &patch(vec![Change::MessageUpsert(Box::new(message))]),
        )
        .unwrap();
    assert_eq!(texts(&sqlite, "Rés"), vec!["resume".to_owned()]);
}

/// `top_hits`: the best of the listed window, as a "Top results" strip ranks them.
mod top_hits {
    use super::*;

    fn kite() -> Filter {
        Filter::Text(TextMatch::Contains("kite".into()))
    }

    fn ids(hits: &[(ThreadSummary, f64)]) -> Vec<ThreadId> {
        hits.iter().map(|(summary, _)| summary.id).collect()
    }

    #[test]
    fn inside_the_window_the_better_match_comes_first() {
        // The strong match is the older thread; newest-first would put it second.
        let held = load(&[
            Mail {
                account: acct_account(),
                subject: "kite kite kite",
                body: Some("ok"),
                secs: 1,
            },
            Mail {
                account: acct_account(),
                subject: "hello",
                body: Some("a kite, among many other words in a longer body"),
                secs: 2,
            },
        ]);
        let hits = ranked(&held.sqlite, &kite(), 5);
        assert_eq!(ids(&hits), [held.threads[0], held.threads[1]]);
        assert!(hits[0].1 > hits[1].1, "higher is better: {hits:?}");
    }

    #[test]
    fn a_strong_match_older_than_the_window_is_left_to_the_list() {
        // The window is the two newest matches. The best match is the oldest and is not in it:
        // bounding the work by the window is the point, and the date-ordered list finds it.
        let held = load(&[
            mail("kite kite kite kite", 1),
            mail("a kite", 2),
            mail("another kite", 3),
        ]);
        for store in stores(&held) {
            let window = listed(store, &kite(), 2);
            let hits = store.top_hits(&kite(), 5, &window, at(10)).unwrap();
            let mut got = ids(&hits);
            got.sort();
            let mut newest = vec![held.threads[1], held.threads[2]];
            newest.sort();
            assert_eq!(got, newest);
        }
    }

    #[test]
    fn at_most_k_and_nothing_to_rank_without_words() {
        let held = load(&[
            mail("kite one", 1),
            mail("kite two", 2),
            mail("kite three", 3),
        ]);
        for store in stores(&held) {
            let all = ranked(store, &kite(), 10);
            assert_eq!(all.len(), 3, "more than the matches is all of them");
            for k in [1, 2] {
                let got = ranked(store, &kite(), k);
                assert_eq!(got.len(), k);
                assert_eq!(got[0].0.id, all[0].0.id, "a limit of {k} keeps the order");
            }
            assert!(ranked(store, &kite(), 0).is_empty());
            assert!(
                ranked(store, &Filter::Account(acct_account()), 5).is_empty(),
                "a filter with no words has nothing to rank by"
            );
            assert!(
                store.top_hits(&kite(), 5, &[], at(10)).unwrap().is_empty(),
                "an empty window has nothing in it to rank"
            );
        }
    }
}

/// A message's attachment names are searched, and its size is what `larger:` and `smaller:`
/// compare: in both stores alike. The SQLite store reads both from what it indexed and stored,
/// the memory store from the message.
#[test]
fn attachment_names_and_sizes_are_searched() {
    let both = Both::new();
    seed_account(&both.sqlite, acct_account(), "me@example.test");
    let raw = both.sqlite.blobs().put(b"raw").unwrap();
    // 2048 bytes of attachment and the bytes of "see attached".
    let message = Message {
        subject: "figures".into(),
        body: Body::Present {
            text: Some("see attached".to_owned()),
            raw,
        },
        attachments: vec![Attachment {
            name: "quarterly-figures.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size: 2048,
            content: PartContent::Held(raw),
            inline: Inline::Attached,
        }],
        ..message(1)
    };
    let size = 2048 + "see attached".len() as u64;
    for (_, store) in both.each() {
        store
            .apply(
                acct_account(),
                &patch(vec![Change::MessageUpsert(Box::new(message.clone()))]),
            )
            .unwrap();
    }
    let cases: Vec<(&str, Filter, usize)> = vec![
        (
            "an attachment's name",
            Filter::Text(TextMatch::Contains("quarterly".into())),
            1,
        ),
        (
            "a word of no part",
            Filter::Text(TextMatch::Contains("annual".into())),
            0,
        ),
        ("larger than a byte under", Filter::Larger(size - 1), 1),
        ("larger than itself", Filter::Larger(size), 0),
        ("smaller than a byte over", Filter::Smaller(size + 1), 1),
        ("smaller than itself", Filter::Smaller(size), 0),
    ];
    for (case, filter, want) in cases {
        let found = both.agree(|store| {
            store
                .threads(&query(filter.clone(), 10), at(0))
                .unwrap()
                .items
                .len()
        });
        assert_eq!(found, want, "{case}");
    }
}
