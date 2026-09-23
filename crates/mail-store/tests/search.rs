//! Prefix terms and the top hits of a listed window.
//!
//! Folding, the range (so `%` and `_` are literals), CJK bigrams, rank order, limits, and the
//! fact that suggestions are not per account. Order is not compared across the two stores; the
//! parity test owns the set, and one case below is where the two orders diverge.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, Term};

const A: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const B: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

struct Mail<'a> {
    account: AccountId,
    subject: &'a str,
    body: Option<&'a str>,
    secs: i64,
}

struct Held {
    _dir: tempfile::TempDir,
    sqlite: SqliteStore,
    memory: MemoryStore,
    threads: Vec<ThreadId>,
}

fn load(mails: &[Mail<'_>]) -> Held {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    let memory = MemoryStore::new();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let mut seen = Vec::new();
    for mail in mails {
        if !seen.contains(&mail.account) {
            seen.push(mail.account);
            sqlite
                .connection()
                .execute(
                    "INSERT INTO accounts (id, address, plan, created_at)
                     VALUES (?1, ?2, '{}', datetime('now'))",
                    rusqlite::params![
                        mail.account.to_string(),
                        format!("{}@example.test", mail.account)
                    ],
                )
                .unwrap();
        }
    }
    let mut threads = Vec::new();
    for (i, mail) in mails.iter().enumerate() {
        let thread = ThreadId::generate();
        threads.push(thread);
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: mail.account,
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: at(mail.secs),
            from: Address {
                name: None,
                email: "q@q.qq".into(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: mail.subject.into(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: None,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: mail.body.map(str::to_owned),
                raw,
            },
            attachments: vec![],
        };
        let patch = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::MessageUpsert(Box::new(message))],
        };
        sqlite.apply(mail.account, &patch).unwrap();
        memory.apply(mail.account, &patch).unwrap();
    }
    Held {
        _dir: dir,
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
    let query = Query {
        filter: filter.clone(),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    };
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
    let held = load(&[Mail {
        account: A,
        subject: "Résumé planned",
        body: None,
        secs: 1,
    }]);
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
        Mail {
            account: A,
            subject: "zz",
            body: None,
            secs: 1,
        },
        Mail {
            account: A,
            subject: "zza",
            body: None,
            secs: 2,
        },
        Mail {
            account: A,
            subject: "zz{",
            body: None,
            secs: 3,
        },
        Mail {
            account: A,
            subject: "z{",
            body: None,
            secs: 4,
        },
        Mail {
            account: A,
            subject: "alpha",
            body: None,
            secs: 5,
        },
        Mail {
            account: A,
            subject: "100%",
            body: None,
            secs: 6,
        },
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
    let held = load(&[Mail {
        account: A,
        subject: "電子郵件",
        body: None,
        secs: 1,
    }]);
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
    let lone = load(&[Mail {
        account: A,
        subject: "電",
        body: None,
        secs: 1,
    }]);
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
    let held = load(&[
        Mail {
            account: A,
            subject: "ac",
            body: None,
            secs: 1,
        },
        Mail {
            account: A,
            subject: "ac",
            body: None,
            secs: 2,
        },
        Mail {
            account: A,
            subject: "aa",
            body: None,
            secs: 3,
        },
        Mail {
            account: A,
            subject: "ab",
            body: None,
            secs: 4,
        },
    ]);
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

#[test]
fn limits_of_zero_one_and_more_than_the_matches() {
    const LIMITS: &[(usize, usize)] = &[(0, 0), (1, 1), (10, 3)];
    let held = load(&[
        Mail {
            account: A,
            subject: "kite",
            body: None,
            secs: 1,
        },
        Mail {
            account: A,
            subject: "kite",
            body: None,
            secs: 2,
        },
        Mail {
            account: A,
            subject: "kite",
            body: None,
            secs: 3,
        },
    ]);
    let filter = Filter::Text(TextMatch::Contains("kite".into()));
    for store in stores(&held) {
        let all = ranked(store, &filter, 10);
        for (limit, n) in LIMITS {
            let got = ranked(store, &filter, *limit);
            assert_eq!(got.len(), *n, "limit {limit}");
            if *n > 0 {
                assert_eq!(got[0].0.id, all[0].0.id);
            }
        }
    }
}

/// The subject that says the word twice outranks one appearance in a long body.
/// Assert the order, not the scores. The better match is the older thread, so a date
/// ordering would come out the other way.
#[test]
fn a_repeated_subject_term_outranks_one_hit_in_a_long_body() {
    let long = format!("{}kite", "padding ".repeat(400));
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    sqlite
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'a@example.test', '{}', datetime('now'))",
            [A.to_string()],
        )
        .unwrap();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let mut ids = Vec::new();
    for (i, (subject, body, secs)) in [("kite kite", "ok", 1i64), ("hello", long.as_str(), 2)]
        .into_iter()
        .enumerate()
    {
        let thread = ThreadId::generate();
        ids.push(thread);
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: A,
            key: MessageKey::Rfc(format!("rank{i}@example.test")),
            date: at(secs),
            from: Address {
                name: None,
                email: "q@q.qq".into(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: None,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some(body.into()),
                raw,
            },
            attachments: vec![],
        };
        sqlite
            .apply(
                A,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .unwrap();
    }
    let filter = Filter::Text(TextMatch::Contains("kite".into()));
    let ranked = ranked(&sqlite, &filter, 10);
    let order: Vec<ThreadId> = ranked.iter().map(|(summary, _)| summary.id).collect();
    assert_eq!(
        order, ids,
        "the short subject should rank ahead of the long body"
    );
}

/// Order is not compared across the two stores.
///
/// SQLite orders by negated bm25, so the older thread (the word twice in the subject) comes
/// first. The memory store scores everything 0.0 and orders by `last_date`, so the newer
/// thread comes first. Asserting those two sequences equal would fail on this corpus, and it
/// would be testing a difference the stores are supposed to have.
#[test]
fn order_is_not_compared_across_the_two_stores() {
    let long = format!("{}kite", "padding ".repeat(400));
    let held = load(&[
        Mail {
            account: A,
            subject: "kite kite",
            body: Some("ok"),
            secs: 1,
        },
        Mail {
            account: A,
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

/// A term that exists only on account B is still suggested. Filtering to account A then
/// drops B's thread, which is why suggesting it costs nothing.
#[test]
fn suggestions_come_from_every_account() {
    let held = load(&[
        Mail {
            account: A,
            subject: "apple harvest",
            body: None,
            secs: 1,
        },
        Mail {
            account: B,
            subject: "xylophone solo",
            body: None,
            secs: 2,
        },
    ]);
    let filter_a = Filter::And(vec![
        Filter::Account(A),
        Filter::Text(TextMatch::Contains("xylophone".into())),
    ]);
    let filter_b = Filter::And(vec![
        Filter::Account(B),
        Filter::Text(TextMatch::Contains("xylophone".into())),
    ]);
    for store in stores(&held) {
        let suggested = texts(store, "xylo");
        assert_eq!(suggested, vec!["xylophone".to_owned()]);
        assert!(ranked(store, &filter_a, 10).is_empty());
        let found = ranked(store, &filter_b, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.account, B);
    }
}

/// Readers are separate connections. The vocab table is created on each of them.
#[test]
fn terms_are_visible_from_a_read_connection() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap();
    sqlite
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'a@example.test', '{}', datetime('now'))",
            [A.to_string()],
        )
        .unwrap();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    sqlite
        .apply(
            A,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(Message {
                    id: MessageId::generate(),
                    thread: ThreadId::generate(),
                    account: A,
                    key: MessageKey::Rfc("reader@example.test".into()),
                    date: at(1),
                    from: Address {
                        name: None,
                        email: "q@q.qq".into(),
                    },
                    reply_to: vec![],
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    subject: "Résumé".into(),
                    in_reply_to: None,
                    references: vec![],
                    rfc_message_id: None,
                    read: ReadState::Unread,
                    star: Star::Unstarred,
                    mailbox: MailboxRole::Inbox,
                    labels: vec![],
                    body: Body::Present { text: None, raw },
                    attachments: vec![],
                }))],
            },
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
                account: A,
                subject: "kite kite kite",
                body: Some("ok"),
                secs: 1,
            },
            Mail {
                account: A,
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
            Mail {
                account: A,
                subject: "kite kite kite kite",
                body: None,
                secs: 1,
            },
            Mail {
                account: A,
                subject: "a kite",
                body: None,
                secs: 2,
            },
            Mail {
                account: A,
                subject: "another kite",
                body: None,
                secs: 3,
            },
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
            Mail {
                account: A,
                subject: "kite one",
                body: None,
                secs: 1,
            },
            Mail {
                account: A,
                subject: "kite two",
                body: None,
                secs: 2,
            },
            Mail {
                account: A,
                subject: "kite three",
                body: None,
                secs: 3,
            },
        ]);
        for store in stores(&held) {
            assert_eq!(ranked(store, &kite(), 2).len(), 2);
            assert!(ranked(store, &kite(), 0).is_empty());
            assert!(
                ranked(store, &Filter::Account(A), 5).is_empty(),
                "a filter with no words has nothing to rank by"
            );
            assert!(
                store.top_hits(&kite(), 5, &[], at(10)).unwrap().is_empty(),
                "an empty window has nothing in it to rank"
            );
        }
    }
}
