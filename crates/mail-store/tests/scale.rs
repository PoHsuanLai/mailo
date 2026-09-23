//! How the store behaves with a real mailbox in it.
//!
//! Every other test here holds two or three messages, which is enough to check *what* a query
//! returns and says nothing about what it costs. The measured maildrop this project was designed
//! around has 2372 messages, a long-lived account has ten times that, and a query that is
//! quadratic in the mailbox is invisible at three rows and unusable at ten thousand.
//!
//! These are not benchmarks and do not assert timings — a loaded CI machine would make that
//! flaky, which is how a performance test gets deleted. They assert the *shape*: that the cost of
//! a page does not grow with the mailbox behind it. A linear scan shows up as a page of 50 taking
//! materially longer at 10,000 rows than at 1,000, and that comparison is stable even on a busy
//! machine.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::time::Instant;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n * 60, 0).unwrap()
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

/// Fill the store with `count` messages, each its own thread, in batches.
fn fill(store: &SqliteStore, count: i64) {
    let raw = store
        .blobs()
        .put(&store.connection(), b"shared body bytes")
        .unwrap();
    for batch in 0..(count / 500) {
        let mut messages = Vec::with_capacity(500);
        for i in 0..500 {
            let n = batch * 500 + i;
            let key = format!("m{n}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread: ThreadId::generate(),
                account: ACCOUNT,
                key: MessageKey::Rfc(key.clone()),
                date: at(n),
                from: Address {
                    name: Some(format!("Sender {}", n % 97)),
                    email: format!("s{}@example.test", n % 97),
                },
                reply_to: vec![],
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: format!("message {n} about quarterly widgets"),
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(key.clone()),
                read: if n % 3 == 0 {
                    ReadState::Unread
                } else {
                    ReadState::Read
                },
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present {
                    text: Some(format!("body of message {n}, mentioning widgets")),
                    raw,
                },
                attachments: vec![],
            };
            messages.push(Fetched {
                remote: RemoteRef::Pop { uidl: key },
                key: message.key.clone(),
                raw,
                message,
            });
        }
        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: MailboxRef {
                        account: ACCOUNT,
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: Some(SyncCursor::Pop),
                    messages,
                    flags: vec![],
                    labels: vec![],
                    label_names: Vec::new(),
                    gone: vec![],
                },
            )
            .unwrap();
    }
}

fn page(limit: u32, filter: Filter) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    }
}

/// Time `f`, taking the best of three so a scheduling hiccup does not decide the result.
fn best_of_three(mut f: impl FnMut()) -> std::time::Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn a_page_of_the_inbox_costs_the_same_at_ten_thousand_as_at_one_thousand() {
    // The property that decides whether this is usable on a real account. `threads` is keyset
    // paginated and ordered by an indexed column, so the first page should cost what a page
    // costs — not what the mailbox costs.
    let (small, _d1) = store();
    fill(&small, 1_000);
    let (large, _d2) = store();
    fill(&large, 10_000);

    assert_eq!(small.count(&Filter::All, at(0)).unwrap(), 1_000);
    assert_eq!(large.count(&Filter::All, at(0)).unwrap(), 10_000);

    let q = page(50, Filter::InMailbox(MailboxRole::Inbox));
    let at_1k = best_of_three(|| {
        small.threads(&q, at(0)).unwrap();
    });
    let at_10k = best_of_three(|| {
        large.threads(&q, at(0)).unwrap();
    });

    eprintln!("first page: 1k={at_1k:?}  10k={at_10k:?}");
    // Ten times the rows must not cost ten times the time. Four is loose enough to survive a
    // busy machine and tight enough that a full scan — which would be ~10x — fails.
    assert!(
        at_10k < at_1k * 4 + std::time::Duration::from_millis(5),
        "a page got dramatically slower with more mail behind it: 1k={at_1k:?} 10k={at_10k:?}"
    );
}

#[test]
fn paging_to_the_end_does_not_get_slower_as_it_goes() {
    // Keyset pagination exists so that page 100 costs what page 1 costs. With OFFSET it would
    // not, and the list would crawl exactly when someone is scrolling back through a year.
    let (store, _dir) = store();
    fill(&store, 10_000);

    let mut query = page(50, Filter::All);
    let first = best_of_three(|| {
        store.threads(&query, at(0)).unwrap();
    });

    // Walk 40 pages in, then measure again from there.
    let mut cursor = None;
    for _ in 0..40 {
        query.page.after = cursor.clone();
        let Ok(p) = store.threads(&query, at(0)) else {
            break;
        };
        if p.next.is_none() {
            break;
        }
        cursor = p.next;
    }
    query.page.after = cursor;
    let deep = best_of_three(|| {
        store.threads(&query, at(0)).unwrap();
    });

    eprintln!("page 1={first:?}  page 41={deep:?}");
    assert!(
        deep < first * 4 + std::time::Duration::from_millis(5),
        "paging got slower the deeper it went: first={first:?} deep={deep:?}"
    );
}

#[test]
fn search_stays_usable_on_a_full_mailbox() {
    // FTS5 is an index; `LIKE '%needle%'` is not. If search ever stopped using the index this is
    // where it shows, because every message here contains the word.
    let (store, _dir) = store();
    fill(&store, 10_000);

    let q = page(50, Filter::Text(TextMatch::Contains("widgets".to_owned())));
    let elapsed = best_of_three(|| {
        store.threads(&q, at(0)).unwrap();
    });
    let found = store.threads(&q, at(0)).unwrap();

    eprintln!("search over 10k: {elapsed:?}, {} hits", found.items.len());
    assert_eq!(found.items.len(), 50, "the page should be full");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "search took {elapsed:?} on 10k messages"
    );
}

#[test]
fn top_hits_of_a_common_word_cost_less_than_listing_it() {
    // The point of `top_hits`: scoring is bounded by the window it is handed, not by how many
    // messages the word is in. Here it is in all ten thousand. The window is listed once, as a
    // caller lists its first page, and only the ranking is timed: a strip that cost as much as
    // the list again would double every keystroke.
    let (store, _dir) = store();
    fill(&store, 10_000);

    let filter = Filter::Text(TextMatch::Contains("widgets".to_owned()));
    let query = page(50, filter.clone());
    let threads = best_of_three(|| {
        store.threads(&query, at(0)).unwrap();
    });
    let window: Vec<ThreadId> = store
        .threads(&page(200, filter.clone()), at(0))
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    let top = best_of_three(|| {
        store.top_hits(&filter, 5, &window, at(0)).unwrap();
    });
    let found = store.top_hits(&filter, 5, &window, at(0)).unwrap();

    eprintln!("top_hits over a window of 200 in 10k: {top:?}   threads(): {threads:?}");
    assert_eq!(found.len(), 5);
    assert!(
        top <= threads + std::time::Duration::from_millis(5),
        "top_hits {top:?} is not bounded by its window: threads() took {threads:?}"
    );
}

#[test]
fn an_unread_count_does_not_read_the_mailbox() {
    // Every sidebar badge runs this on every revision. A count that scans is a count that makes
    // the whole window pause each time a message arrives.
    let (store, _dir) = store();
    fill(&store, 10_000);

    let filter = Filter::And(vec![
        Filter::InMailbox(MailboxRole::Inbox),
        Filter::Read(ReadState::Unread),
    ]);
    let elapsed = best_of_three(|| {
        store.count(&filter, at(0)).unwrap();
    });
    eprintln!("unread count over 10k: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "counting unread took {elapsed:?}"
    );
}

/// How long a first sync spends writing, which is the part a user watches.
mod ingest_throughput {
    use super::*;

    /// One batch of `count` messages, all in distinct threads.
    fn batch(store: &SqliteStore, from: i64, count: i64, thread: Option<ThreadId>) -> Ingest {
        let raw = store
            .blobs()
            .put(&store.connection(), b"shared body bytes")
            .unwrap();
        let mut messages = Vec::with_capacity(count as usize);
        for i in 0..count {
            let n = from + i;
            let key = format!("t{n}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread: thread.unwrap_or_else(ThreadId::generate),
                account: ACCOUNT,
                key: MessageKey::Rfc(key.clone()),
                date: at(n),
                from: Address {
                    name: None,
                    email: "sender@example.test".to_owned(),
                },
                reply_to: vec![],
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: format!("subject {n}"),
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(key.clone()),
                read: ReadState::Unread,
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present {
                    text: Some(format!("body {n}")),
                    raw,
                },
                attachments: vec![],
            };
            messages.push(Fetched {
                remote: RemoteRef::Pop { uidl: key },
                key: message.key.clone(),
                raw,
                message,
            });
        }
        Ingest {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            validity: UidValidity::Same,
            cursor: Some(SyncCursor::Pop),
            messages,
            flags: vec![],
            labels: vec![],
            label_names: Vec::new(),
            gone: vec![],
        }
    }

    #[test]
    fn a_first_sync_of_a_real_maildrop_is_not_something_you_wait_out() {
        // 2372 is the maildrop this project was measured against. A first sync fetches in
        // batches, so this is the store's share of that time — not the network's.
        let (store, _dir) = store();
        let start = Instant::now();
        for b in 0..5 {
            store
                .ingest(ACCOUNT, batch(&store, b * 500, 500, None))
                .unwrap();
        }
        let elapsed = start.elapsed();
        let each = elapsed / 2_500;
        eprintln!("2500 messages absorbed in {elapsed:?} ({each:?} each)");

        assert_eq!(store.count(&Filter::All, at(0)).unwrap(), 2_500);
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "absorbing a real maildrop took {elapsed:?}"
        );
    }

    #[test]
    fn absorbing_into_a_long_thread_does_not_get_slower_as_the_thread_grows() {
        // `refresh_summary` reloads every message of a touched thread to recompute its summary.
        // That is fine per ingest and quadratic if a mailing-list thread is absorbed one batch
        // at a time — which is exactly how a sync arrives.
        let (store, _dir) = store();
        let thread = ThreadId::generate();

        let first = {
            let start = Instant::now();
            store
                .ingest(ACCOUNT, batch(&store, 0, 200, Some(thread)))
                .unwrap();
            start.elapsed()
        };
        // Nine more batches into the same thread, so it ends at 2000 messages.
        for b in 1..9 {
            store
                .ingest(ACCOUNT, batch(&store, b * 200, 200, Some(thread)))
                .unwrap();
        }
        let last = {
            let start = Instant::now();
            store
                .ingest(ACCOUNT, batch(&store, 9 * 200, 200, Some(thread)))
                .unwrap();
            start.elapsed()
        };

        eprintln!("batch into an empty thread: {first:?}; into a 1800-message thread: {last:?}");
        assert!(
            last < first * 8 + std::time::Duration::from_millis(50),
            "absorbing got much slower as the thread grew: first={first:?} last={last:?}"
        );
    }
}

/// The query shapes the search language made reachable.
///
/// F118 turned `from:ada is:unread after:2026-01-01` into `Filter::And` of three clauses, which
/// nothing in this application had ever asked the store for. A query language that produces
/// shapes the SQL builder answers by reading the whole mailbox is a slower search than the one
/// it replaced, so the shapes are measured rather than assumed.
mod the_search_language {
    use super::*;

    fn timed(store: &SqliteStore, name: &str, filter: Filter) -> std::time::Duration {
        let q = page(50, filter);
        let elapsed = best_of_three(|| {
            store.threads(&q, at(0)).unwrap();
        });
        let hits = store.threads(&q, at(0)).unwrap().items.len();
        eprintln!("{name:<44} {elapsed:>10?}  {hits} hits");
        elapsed
    }

    #[test]
    fn every_shape_the_language_can_build_stays_usable() {
        let (store, _dir) = store();
        fill(&store, 10_000);

        let cases: Vec<(&str, Filter)> = vec![
            (
                "from:s1",
                Filter::From(TextMatch::Contains("s1@example.test".to_owned())),
            ),
            (
                "subject:widgets",
                Filter::Subject(TextMatch::Contains("widgets".to_owned())),
            ),
            ("is:unread", Filter::Read(ReadState::Unread)),
            ("is:starred", Filter::Starred(Star::Starred)),
            ("has:attachment", Filter::HasAttachment),
            ("is:pinned", Filter::Pinned),
            ("is:snoozed", Filter::Snoozed),
            (
                "after:…",
                Filter::Date(DateRange {
                    from: Some(at(5_000)),
                    to: None,
                }),
            ),
            (
                "-from:s1",
                Filter::Not(Box::new(Filter::From(TextMatch::Contains(
                    "s1@example.test".to_owned(),
                )))),
            ),
            (
                "from:s1 is:unread",
                Filter::And(vec![
                    Filter::From(TextMatch::Contains("s1@example.test".to_owned())),
                    Filter::Read(ReadState::Unread),
                ]),
            ),
            (
                "from:s1 widgets after:… is:unread",
                Filter::And(vec![
                    Filter::From(TextMatch::Contains("s1@example.test".to_owned())),
                    Filter::Text(TextMatch::Contains("widgets".to_owned())),
                    Filter::Date(DateRange {
                        from: Some(at(5_000)),
                        to: None,
                    }),
                    Filter::Read(ReadState::Unread),
                ]),
            ),
        ];

        let mut slowest = std::time::Duration::ZERO;
        let mut worst = "";
        for (name, filter) in cases {
            let elapsed = timed(&store, name, filter);
            if elapsed > slowest {
                slowest = elapsed;
                worst = name;
            }
        }
        // Generous against a loaded machine and still far under the threshold where a search
        // box stops feeling immediate. What this catches is a shape answered by reading the
        // mailbox — which is hundreds of milliseconds at this size, not tens.
        assert!(
            slowest < std::time::Duration::from_millis(500),
            "{worst:?} took {slowest:?} on 10k messages"
        );
    }

    #[test]
    fn adding_a_term_does_not_cost_more_than_the_terms_it_narrows() {
        // The property that makes a query language safe to offer: each clause you add finds
        // less, so it must not take longer. A builder that evaluated clauses independently and
        // intersected afterwards would fail this.
        let (store, _dir) = store();
        fill(&store, 10_000);

        let one = page(
            50,
            Filter::From(TextMatch::Contains("s1@example.test".to_owned())),
        );
        let three = page(
            50,
            Filter::And(vec![
                Filter::From(TextMatch::Contains("s1@example.test".to_owned())),
                Filter::Read(ReadState::Unread),
                Filter::Date(DateRange {
                    from: Some(at(5_000)),
                    to: None,
                }),
            ]),
        );
        let first = best_of_three(|| {
            store.threads(&one, at(0)).unwrap();
        });
        let narrowed = best_of_three(|| {
            store.threads(&three, at(0)).unwrap();
        });
        eprintln!("one clause {first:?}, three clauses {narrowed:?}");
        assert!(
            narrowed < first * 4,
            "narrowing made it slower: one={first:?} three={narrowed:?}"
        );
    }
}
