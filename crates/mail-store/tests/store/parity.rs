//! `Filter::fit` and `sql::compile` are two implementations of one semantics.
//!
//! They will diverge. The symptom is not a crash — it is search quietly missing a message, which
//! nobody reports as a bug because nobody knows the message was there. This is the only thing
//! standing between that and a release.
//!
//! The corpus reaches past Latin on purpose. Both sides tokenize with the same function now, and
//! `tests/store/fold_table.rs` proves SQLite leaves its tokens alone for every code point; the words
//! below are the cases that proof is about — folds across scripts, marks that join a word and
//! marks that end one — exercised end to end through both implementations.

use crate::support::*;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::testing::seed_account;
use mail_store::{MemoryStore, SqliteStore, Store};
use proptest::prelude::*;
use std::collections::BTreeSet;

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
    "校園郵件",
    "郵件",
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
    /// Which of [`FOLDERS`] hold a copy on the server: none, one, or several.
    folders: Vec<bool>,
    /// A queued move into one of [`MOVES`], not yet confirmed by the server.
    moving: Option<usize>,
    /// A queued copy into one of [`MOVES`], queued before the move, not yet confirmed either.
    copying: Option<usize>,
    date: i64,
    /// An attachment named after one of [`WORDS`], of this many bytes, still on the server.
    attached: Option<(usize, u64)>,
    /// Copied to an address built from one of [`WORDS`], as the sender's is.
    cc: Option<usize>,
    /// Whether the sender is on the VIP list.
    vip: bool,
}

/// A sender's or a recipient's address, built from one of [`WORDS`].
fn address_of(word: usize) -> String {
    format!("{}@example.test", WORDS[word].replace(['%', ' '], ""))
}

/// Folders a queued `File` moves, or a queued `Copy` copies, a message into: one it may already
/// be held in, and one it is not. Only the second takes a move out of `InFolder` for the folder
/// it is in, and lists a copy in `InFolder` for the folder it goes to.
const MOVES: &[(&str, LabelId)] = &[
    (
        "Projects/2026",
        LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1")),
    ),
    (
        "Elsewhere",
        LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c2")),
    ),
];

/// The roles the account's server reported. The non-ASCII folder is Trash, so a message held
/// there is filed as Trash and one held there as anything else is not in it; the second entry
/// for the same path is never the one consulted.
fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(vec![
            ("收件匣/報告".to_owned(), MailboxRole::Trash),
            ("收件匣/報告".to_owned(), MailboxRole::Spam),
        ]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: at(0),
    }
}

/// Server mailboxes a message can have an address in. `INBOX` beside a user folder, a nested
/// one, and one whose name needs modified UTF-7 on the wire and is stored decoded.
const FOLDERS: &[&str] = &["INBOX", "Projects/2026", "收件匣/報告"];

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
        prop::collection::vec(any::<bool>(), FOLDERS.len()..=FOLDERS.len()),
        (
            prop::option::of(0..MOVES.len()),
            prop::option::of(0..MOVES.len()),
            0i64..4_000,
            prop::option::of((0..WORDS.len(), 0u64..60)),
            prop::option::of(0..WORDS.len()),
            any::<bool>(),
        ),
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
                folders,
                (moving, copying, date, attached, cc, vip),
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
                    folders,
                    moving,
                    copying,
                    date,
                    attached,
                    cc,
                    vip,
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
        Just(Filter::Account(acct_account())),
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
        // The other account too, so `InFolder` is seen to scope by account as well as by path.
        (0..FOLDERS.len(), any::<bool>()).prop_map(|(i, mine)| Filter::InFolder(MailboxRef {
            account: if mine { acct_account() } else { acct_other() },
            path: FOLDERS[i].to_owned(),
        })),
        text.clone().prop_map(Filter::From),
        text.clone().prop_map(Filter::To),
        text.clone().prop_map(Filter::Cc),
        // The address alone, in any ASCII case, which `lower` and `eq_ignore_ascii_case` fold alike.
        (0..WORDS.len(), any::<bool>()).prop_map(|(i, shout)| Filter::FromAddress(if shout {
            address_of(i).to_ascii_uppercase()
        } else {
            address_of(i)
        })),
        Just(Filter::Vip),
        text.clone().prop_map(Filter::Subject),
        text.prop_map(Filter::Text),
        (0u64..80).prop_map(Filter::Larger),
        (0u64..80).prop_map(Filter::Smaller),
        (0u32..4).prop_map(Filter::NewerThan),
        (0u32..4).prop_map(Filter::OlderThan),
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
    seed_account(&sqlite, acct_account(), "me@example.test");
    let memory = MemoryStore::new();
    sqlite.put_caps(acct_account(), &caps(), at(0)).unwrap();
    memory.put_caps(acct_account(), &caps(), at(0)).unwrap();

    let named = [(LABEL_A, "work"), (LABEL_B, "personal")]
        .into_iter()
        .chain(MOVES.iter().map(|(path, id)| (*id, *path)));
    for (id, name) in named {
        let label = Label {
            id,
            account: acct_account(),
            name: name.to_owned(),
            color: None,
            origin: LabelOrigin::User,
        };
        let patch = patch(vec![Change::LabelUpsert(label)]);
        sqlite.apply(acct_account(), &patch).unwrap();
        memory.apply(acct_account(), &patch).unwrap();
    }

    let mut held: Vec<(Message, &Spec)> = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        let body = words(&s.body);
        // The blob must exist in SQLite before a message can reference it (foreign key), and
        // both stores must agree on the id.
        let raw = sqlite
            .blobs()
            .put(format!("raw {i} {body}").as_bytes())
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
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: at(s.date),
            from: Address {
                name: s.from_name.map(|n| WORDS[n].to_owned()),
                email: address_of(s.from_local),
            },
            to: vec![Address {
                name: None,
                email: address_of(s.to_local),
            }],
            cc: s
                .cc
                .map(|word| Address {
                    name: None,
                    email: address_of(word),
                })
                .into_iter()
                .collect(),
            subject: words(&s.subject),
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
            attachments: s
                .attached
                .map(|(word, size)| Attachment {
                    name: format!("{}.pdf", WORDS[word]),
                    mime: "application/pdf".to_owned(),
                    size,
                    content: PartContent::Remote {
                        section: Section::of(&[2]).unwrap(),
                    },
                    inline: Inline::Attached,
                })
                .into_iter()
                .collect(),
            ..message(0)
        };
        let patch = patch(vec![Change::MessageUpsert(Box::new(message.clone()))]);
        sqlite.apply(acct_account(), &patch).unwrap();
        memory.apply(acct_account(), &patch).unwrap();
        if s.vip {
            sqlite.put_vip(&message.from.email, None).unwrap();
            memory.put_vip(&message.from.email, None).unwrap();
        }
        held.push((message, s));
    }

    // Server addresses, through `ingest` as a sync writes them: each message arriving again
    // from every folder that holds a copy, matched to the one already held by its key.
    for (f, path) in FOLDERS.iter().enumerate() {
        let messages: Vec<Fetched> = held
            .iter()
            .enumerate()
            .filter(|(_, (_, s))| s.folders[f])
            .map(|(i, (message, _))| Fetched {
                remote: RemoteRef::Imap {
                    mailbox: (*path).to_owned(),
                    uidvalidity: 7,
                    uid: i as u32 + 1,
                },
                key: message.key.clone(),
                raw: match &message.body {
                    Body::Present { raw, .. } => *raw,
                    Body::Absent => unreachable!("every generated message has a body"),
                },
                message: message.clone(),
            })
            .collect();
        let ingest = Ingest {
            mailbox: MailboxRef {
                account: acct_account(),
                path: (*path).to_owned(),
            },
            validity: UidValidity::Same,
            cursor: None,
            messages,
            flags: Vec::new(),
            flag_colors: Vec::new(),
            labels: Vec::new(),
            label_names: Vec::new(),
            provider_threads: Vec::new(),
            gone: Vec::new(),
        };
        sqlite.ingest(acct_account(), ingest.clone()).unwrap();
        memory.ingest(acct_account(), ingest).unwrap();
    }

    // Copies and moves queued and not yet confirmed, as the window or a rule queues them: the op
    // applied here, its remote half in the outbox. A message with no address queues nothing.
    let queued = held.iter().flat_map(|(message, s)| {
        [
            s.copying.map(|to| (message, Op::Copy(MOVES[to].1))),
            s.moving.map(|to| (message, Op::File(MOVES[to].1))),
        ]
        .into_iter()
        .flatten()
    });
    for (message, op) in queued {
        for store in [&sqlite as &dyn Store, &memory] {
            let thread = store.thread(message.thread).unwrap();
            let messages: Vec<Message> = thread
                .messages
                .iter()
                .map(|id| store.message(*id).unwrap())
                .collect();
            let applied = op.apply(
                &Target::Messages(vec![message.id]),
                &thread,
                &messages,
                &caps(),
                now(),
            );
            store.apply(acct_account(), &applied.forward).unwrap();
            if let Some(intent) = applied.remote {
                store
                    .enqueue(acct_account(), intent, &applied.inverse, now())
                    .unwrap();
            }
        }
    }
    Both {
        sqlite,
        memory,
        _dir: dir,
    }
}

/// ASCII prefixes drawn from the corpus words. Empty is included: both stores must return
/// nothing, not the vocabulary.
const ASCII_PREFIXES: &[&str] = &["", "a", "re", "lun", "fr", "50", "cafe", "plain", "z", "x"];

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

fn term_set(store: &dyn Store, prefix: &str) -> BTreeSet<String> {
    store
        .terms_with_prefix(prefix, 1000)
        .expect("terms must not error")
        .into_iter()
        .map(|term| term.text)
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

    /// `top_hits` is at most `k` threads, every one of them in the window it was given — the
    /// first `window` that `threads` returns — once each, in both stores.
    #[test]
    fn top_hits_come_from_the_newest_window_of_the_list(
        specs in prop::collection::vec(spec(), 1..8),
        f in filter(),
        k in 0usize..4,
        window in 0usize..6,
    ) {
        let both = build(&specs);
        for store in [&both.sqlite as &dyn Store, &both.memory] {
            let listed = store
                .threads(
                    &Query {
                        filter: f.clone(),
                        sort: Sort { property: Property::Date, dir: SortDir::Desc },
                        page: PageReq { after: None, limit: window as u32 },
                    },
                    now(),
                )
                .expect("threads");
            let newest: BTreeSet<ThreadId> = listed.items.iter().map(|s| s.id).collect();
            let given: Vec<ThreadId> = listed.items.iter().map(|s| s.id).collect();
            let hits = store.top_hits(&f, k, &given, now()).expect("top_hits");
            prop_assert!(hits.len() <= k, "{} hits for k = {}", hits.len(), k);
            let distinct: BTreeSet<ThreadId> = hits.iter().map(|(s, _)| s.id).collect();
            prop_assert_eq!(distinct.len(), hits.len(), "duplicate thread in {:?}", f);
            for (summary, _) in &hits {
                prop_assert!(newest.contains(&summary.id), "{:?} is outside the window for {:?}", summary.id, f);
            }
        }
    }

    /// Paging through the list a few rows at a time, under every sort, reaches each thread once
    /// and in the order one page of all of them has, in both stores, and the stores agree on
    /// that order.
    #[test]
    fn paging_under_every_sort_reaches_each_thread_once(
        specs in prop::collection::vec(spec(), 1..8),
        limit in 1u32..4,
    ) {
        let both = build(&specs);
        let properties = [
            Property::Date,
            Property::Subject,
            Property::From,
            Property::Sender,
            Property::Size,
            Property::Attachments,
            Property::Pin,
            Property::Unread,
            Property::Starred,
        ];
        let mut orders: Vec<Vec<Vec<ThreadId>>> = Vec::new();
        for store in [&both.sqlite as &dyn Store, &both.memory] {
            let mut order = Vec::new();
            for property in properties {
                for dir in [SortDir::Asc, SortDir::Desc] {
                    let query = |after, limit| Query {
                        filter: Filter::All,
                        sort: Sort { property, dir },
                        page: PageReq { after, limit },
                    };
                    let whole: Vec<ThreadId> = store
                        .threads(&query(None, 1000), now())
                        .expect("threads")
                        .items
                        .iter()
                        .map(|s| s.id)
                        .collect();
                    let mut paged = Vec::new();
                    let mut after = None;
                    loop {
                        let page = store.threads(&query(after, limit), now()).expect("threads");
                        paged.extend(page.items.iter().map(|s| s.id));
                        prop_assert!(paged.len() <= whole.len(), "{:?} {:?} repeats rows", property, dir);
                        match page.next {
                            Some(next) => after = Some(next),
                            None => break,
                        }
                    }
                    prop_assert_eq!(&paged, &whole, "{:?} {:?}", property, dir);
                    order.push(whole);
                }
            }
            orders.push(order);
        }
        prop_assert_eq!(&orders[0], &orders[1], "SQLite and memory sort apart");
    }

    /// Term sets for an ASCII prefix. Both stores fold with `search_tokens` and count a term
    /// once per message, so the texts agree. Order is not the question here.
    #[test]
    fn ascii_prefix_term_sets_agree(
        specs in prop::collection::vec(spec(), 1..8),
        prefix in prop::sample::select(ASCII_PREFIXES),
    ) {
        let both = build(&specs);
        prop_assert_eq!(
            term_set(&both.sqlite, prefix),
            term_set(&both.memory, prefix),
            "prefix {:?}",
            prefix
        );
    }
}
