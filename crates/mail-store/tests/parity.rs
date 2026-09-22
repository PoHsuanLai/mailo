//! `Filter::fit` and `sql::compile` are two implementations of one semantics.
//!
//! They will diverge. The symptom is not a crash — it is search quietly missing a message, which
//! nobody reports as a bug because nobody knows the message was there. This is the only thing
//! standing between that and a release.
//!
//! The corpus reaches past Latin on purpose. Both sides tokenize with the same function now, and
//! `tests/fold_table.rs` proves SQLite leaves its tokens alone for every code point; the words
//! below are the cases that proof is about — folds across scripts, marks that join a word and
//! marks that end one — exercised end to end through both implementations.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};
use proptest::prelude::*;
use std::collections::BTreeSet;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const LABEL_A: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const LABEL_B: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

fn at(secs: i64) -> DateTime<Utc> {
    // Whole seconds: from_time writes fixed nine-digit nanoseconds, and sub-second corpora only
    // test the formatter, not the semantics.
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

fn now() -> DateTime<Utc> {
    at(5_000)
}

/// Words chosen in pairs that search must treat alike, or must not: accented and plain Latin,
/// final and medial sigma, a long s, a decomposed accent, pointed and unpointed Hebrew, Cyrillic
/// case, and Chinese that has to be split into bigrams to be found at all.
const WORDS: &[&str] = &[
    "lunch",
    "friday",
    "résumé",
    "resume",
    "ada",
    "lovelace",
    "invoice",
    "50%",
    "café",
    "meeting",
    "Ünicode",
    "plain",
    "re",
    "fwd",
    "σοφίας",
    "ΣΟΦΙΑΣ",
    "ſtraße",
    "strasse",
    "e\u{301}te",
    "ête",
    "שָׁלוֹם",
    "שלום",
    "Москва",
    "москва",
    "臺大計中",
    "計中",
    "µs",
    "μs",
];

#[derive(Debug, Clone)]
struct Spec {
    thread: u8,
    subject: Vec<usize>,
    body: Vec<usize>,
    from_name: Option<usize>,
    from_local: usize,
    to_local: usize,
    read: bool,
    star: bool,
    mailbox: u8,
    labels: Vec<bool>,
    date: i64,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        0u8..4,
        prop::collection::vec(0..WORDS.len(), 1..4),
        prop::collection::vec(0..WORDS.len(), 1..4),
        prop::option::of(0..WORDS.len()),
        0..WORDS.len(),
        0..WORDS.len(),
        any::<bool>(),
        any::<bool>(),
        0u8..6,
        prop::collection::vec(any::<bool>(), 2..=2),
        0i64..4_000,
    )
        .prop_map(
            |(
                thread,
                subject,
                body,
                from_name,
                from_local,
                to_local,
                read,
                star,
                mailbox,
                labels,
                date,
            )| {
                Spec {
                    thread,
                    subject,
                    body,
                    from_name,
                    from_local,
                    to_local,
                    read,
                    star,
                    mailbox,
                    labels,
                    date,
                }
            },
        )
}

fn role(n: u8) -> MailboxRole {
    MailboxRole::ALL[usize::from(n) % MailboxRole::ALL.len()]
}

fn words(idx: &[usize]) -> String {
    idx.iter().map(|i| WORDS[*i]).collect::<Vec<_>>().join(" ")
}

/// Filters, recursively, over every variant the two sides both claim to implement.
fn filter() -> impl Strategy<Value = Filter> {
    let text = prop_oneof![
        (0..WORDS.len()).prop_map(|i| TextMatch::Contains(WORDS[i].to_owned())),
        (0..WORDS.len()).prop_map(|i| TextMatch::Exact(WORDS[i].to_owned())),
        prop::collection::vec(0..WORDS.len(), 2..3).prop_map(|v| TextMatch::Contains(words(&v))),
    ];
    let leaf = prop_oneof![
        Just(Filter::All),
        Just(Filter::Nothing),
        Just(Filter::HasAttachment),
        Just(Filter::Snoozed),
        Just(Filter::SnoozeDue),
        Just(Filter::Pinned),
        Just(Filter::Account(ACCOUNT)),
        (0u8..6).prop_map(|n| Filter::InMailbox(role(n))),
        any::<bool>().prop_map(|b| Filter::Read(if b {
            ReadState::Read
        } else {
            ReadState::Unread
        })),
        any::<bool>().prop_map(|b| Filter::Starred(if b {
            Star::Starred
        } else {
            Star::Unstarred
        })),
        any::<bool>().prop_map(|b| Filter::HasLabel(if b { LABEL_A } else { LABEL_B })),
        text.clone().prop_map(Filter::From),
        text.clone().prop_map(Filter::To),
        text.clone().prop_map(Filter::Subject),
        text.prop_map(Filter::Text),
        (0i64..4_000, 0i64..4_000).prop_map(|(a, b)| Filter::Date(DateRange {
            from: Some(at(a.min(b))),
            to: Some(at(a.max(b) + 1)),
        })),
    ];
    leaf.prop_recursive(3, 8, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Filter::And),
            prop::collection::vec(inner.clone(), 0..3).prop_map(Filter::Or),
            inner.prop_map(|f| Filter::Not(Box::new(f))),
        ]
    })
}

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

fn build(specs: &[Spec]) -> Both {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    sqlite
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    let memory = MemoryStore::new();

    for (id, name) in [(LABEL_A, "work"), (LABEL_B, "personal")] {
        let label = Label {
            id,
            account: ACCOUNT,
            name: name.to_owned(),
            color: None,
            origin: LabelOrigin::User,
        };
        let patch = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::LabelUpsert(label)],
        };
        sqlite.apply(ACCOUNT, &patch).unwrap();
        memory.apply(ACCOUNT, &patch).unwrap();
    }

    for (i, s) in specs.iter().enumerate() {
        let body = words(&s.body);
        // The blob must exist in SQLite before a message can reference it (foreign key), and
        // both stores must agree on the id.
        let raw = sqlite
            .blobs()
            .put(&sqlite.connection(), format!("raw {i} {body}").as_bytes())
            .unwrap();
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + u128::from(s.thread)));
        let labels: Vec<LabelId> = [LABEL_A, LABEL_B]
            .iter()
            .zip(&s.labels)
            .filter_map(|(l, keep)| keep.then_some(*l))
            .collect();
        let message = Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + i as u128)),
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: at(s.date),
            from: Address {
                name: s.from_name.map(|n| WORDS[n].to_owned()),
                email: format!(
                    "{}@example.test",
                    WORDS[s.from_local].replace(['%', ' '], "")
                ),
            },
            reply_to: vec![],
            to: vec![Address {
                name: None,
                email: format!("{}@example.test", WORDS[s.to_local].replace(['%', ' '], "")),
            }],
            cc: vec![],
            bcc: vec![],
            subject: words(&s.subject),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{i}@example.test")),
            read: if s.read {
                ReadState::Read
            } else {
                ReadState::Unread
            },
            star: if s.star {
                Star::Starred
            } else {
                Star::Unstarred
            },
            mailbox: role(s.mailbox),
            labels,
            body: Body::Present {
                text: Some(body),
                raw,
            },
            attachments: vec![],
        };
        let patch = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::MessageUpsert(Box::new(message))],
        };
        sqlite.apply(ACCOUNT, &patch).unwrap();
        memory.apply(ACCOUNT, &patch).unwrap();
    }
    Both {
        sqlite,
        memory,
        _dir: dir,
    }
}

fn ids(store: &dyn Store, f: &Filter) -> BTreeSet<ThreadId> {
    let query = Query {
        filter: f.clone(),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        // Larger than any generated corpus: this compares result SETS, and pagination has its
        // own test. A short page here would compare two different questions.
        page: PageReq {
            after: None,
            limit: 1000,
        },
    };
    store
        .threads(&query, now())
        .expect("query must not error")
        .items
        .into_iter()
        .map(|s| s.id)
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// The invariant: for every filter and every corpus, the two implementations select the
    /// same threads. A failure here is a real divergence, and the shrunk case names it.
    #[test]
    fn fit_and_sql_select_the_same_threads(
        specs in prop::collection::vec(spec(), 1..8),
        f in filter(),
    ) {
        let both = build(&specs);
        let from_fit = ids(&both.memory, &f);
        let from_sql = ids(&both.sqlite, &f);
        prop_assert_eq!(
            &from_fit,
            &from_sql,
            "filter {:?}\n  fit selected {:?}\n  sql selected {:?}",
            f, from_fit, from_sql
        );
    }

    /// `count` must agree with what `threads` actually returns, or a sidebar badge lies.
    #[test]
    fn count_agrees_with_the_rows_it_counts(
        specs in prop::collection::vec(spec(), 1..8),
        f in filter(),
    ) {
        let both = build(&specs);
        let rows = ids(&both.sqlite, &f).len() as u64;
        let counted = both.sqlite.count(&f, now()).expect("count must not error");
        prop_assert_eq!(counted, rows);
    }
}
