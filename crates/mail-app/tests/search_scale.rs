//! What one keystroke in the search box costs over a mailbox that is actually large.
//!
//! Sibling to `reader_scale.rs`. Every other search test holds a handful of threads, which says
//! what a query returns and nothing about what it costs. The list box runs
//! [`mail_app::search::search_list`] once the box is still — parse, prefix expansion through the
//! index vocabulary, one page of matches in date order (`listed`), and the "Top results" strip
//! ranked inside a window of the newest matches (`top`) — so that whole pipeline is what is
//! timed here, one character at a time, the way `uidvalidity` is actually typed.
//!
//! The budget is 30 ms at the 95th percentile, in a release build, over 50,000 messages.
//!
//! The words are drawn Zipf-distributed from an 8,000-word vocabulary, which is how text is
//! distributed: a few words (`the`, `us`, `update`) are in a large share of messages, and a
//! technical word like `uid` is in a few hundred. A uniform draw from a short list would make
//! every word a stop word, which no mailbox is. What a frequent word costs in all is the
//! store's `threads()` and is printed, not asserted; what is asserted for it is that the strip
//! adds at most half again to the list it sits on, so ranking never costs what ranking every
//! match did.
//!
//! Run it with:
//!
//! ```text
//! cargo test -p mail-app --release --test search_scale -- --ignored --nocapture
//! ```

use chrono::{DateTime, TimeZone, Utc};
use mail_app::search::{Affinity, Source, Term, first, search_list};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::cell::Cell;
use std::time::{Duration, Instant};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

/// The mailbox. Fifty thousand is a long-lived account, and twenty times the measured maildrop
/// this project was designed around.
const MESSAGES: u64 = 50_000;
const BATCH: u64 = 500;

/// One message in this many is about UIDVALIDITY, so the typed word has real hits to rank.
const ABOUT_UIDVALIDITY: u64 = 400;

/// Rounds of typing the word. Eleven keystrokes each, so five rounds is 55 samples, enough for
/// a 95th percentile that is not just the maximum.
const ROUNDS: usize = 5;

const BUDGET_P95: Duration = Duration::from_millis(30);

const TYPED: &str = "uidvalidity";

const VOCABULARY: usize = 8_000;

/// Real words at the rank they are given, most frequent first. Several start with `u`, as they
/// do in English, so the first keystroke meets the commonest words it could complete to.
const COMMON: &[&str] = &[
    "the",
    "to",
    "and",
    "a",
    "of",
    "you",
    "for",
    "is",
    "in",
    "on",
    "this",
    "it",
    "we",
    "your",
    "us",
    "with",
    "be",
    "are",
    "up",
    "at",
    "not",
    "have",
    "from",
    "use",
    "by",
    "if",
    "can",
    "will",
    "all",
    "our",
    "more",
    "please",
    "new",
    "update",
    "time",
    "one",
    "about",
    "email",
    "unsubscribe",
    "until",
    "under",
    "message",
    "user",
    "usually",
    "server",
    "thread",
];

/// Rarer words, and the rank each sits at. The typed word's neighbours are technical terms.
const PLACED: &[(&str, usize)] = &[
    ("uidvalidity", 900),
    ("uid", 1_500),
    ("uidl", 2_500),
    ("uidnext", 4_000),
    ("unsolicited", 3_000),
    ("uint", 6_000),
];

fn at(n: u64) -> DateTime<Utc> {
    let offset = i64::try_from(n).unwrap_or(i64::MAX) * 60;
    Utc.timestamp_opt(1_700_000_000 + offset, 0).unwrap()
}

/// The vocabulary in rank order: the common words, the placed ones at their ranks, and
/// invented tokens spread across the alphabet in between.
fn vocabulary() -> Vec<String> {
    let mut words: Vec<String> = COMMON.iter().map(|word| (*word).to_owned()).collect();
    let mut token = 0usize;
    while words.len() < VOCABULARY {
        let rank = words.len();
        match PLACED.iter().find(|(_, at)| *at == rank) {
            Some((word, _)) => words.push((*word).to_owned()),
            None => {
                let letter = char::from(b'a' + u8::try_from(token % 26).unwrap_or(0));
                words.push(format!("{letter}x{token}"));
                token += 1;
            }
        }
    }
    words
}

/// A deterministic Zipf sampler, so every run indexes the same mailbox.
struct Zipf {
    state: u64,
    words: Vec<String>,
    /// Cumulative `1/rank`, for a binary search.
    cumulative: Vec<f64>,
}

impl Zipf {
    fn new() -> Self {
        let words = vocabulary();
        let mut total = 0.0;
        let cumulative = (1..=words.len())
            .map(|rank| {
                total += 1.0 / rank as f64;
                total
            })
            .collect();
        Self {
            state: 0x5eed,
            words,
            cumulative,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 11) as f64 / (1u64 << 53) as f64
    }

    fn word(&mut self) -> &str {
        let total = self.cumulative.last().copied().unwrap_or(1.0);
        let target = self.uniform() * total;
        let index = self
            .cumulative
            .partition_point(|sum| *sum < target)
            .min(self.words.len() - 1);
        &self.words[index]
    }

    fn words(&mut self, count: usize) -> String {
        (0..count)
            .map(|_| self.word().to_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A file-backed store in `dir`, the way the window opens one, with one account.
fn store(dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    store
}

/// Ingest `MESSAGES` messages through the store API, in batches, each its own thread.
fn fill(store: &SqliteStore) {
    // One raw blob shared by every message: the index reads `text`, and fifty thousand copies
    // of the same bytes would measure the disk, not the search.
    let raw = store
        .blobs()
        .put(&store.connection(), b"shared body bytes")
        .unwrap();
    let mut zipf = Zipf::new();
    for batch in 0..MESSAGES / BATCH {
        let messages = (0..BATCH)
            .map(|i| {
                let n = batch * BATCH + i;
                let mut subject = zipf.words(6);
                let text = zipf.words(40);
                if n.is_multiple_of(ABOUT_UIDVALIDITY) {
                    subject = format!("UIDVALIDITY changed on {subject}");
                }
                let key = format!("m{n}@example.test");
                let message = Message {
                    id: MessageId::generate(),
                    thread: ThreadId::generate(),
                    account: ACCOUNT,
                    key: MessageKey::Rfc(key.clone()),
                    date: at(n),
                    from: Address {
                        name: Some(format!("Sender {}", n % 211)),
                        email: format!("s{}@example.test", n % 211),
                    },
                    reply_to: vec![],
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    subject,
                    in_reply_to: None,
                    references: vec![],
                    rfc_message_id: Some(key.clone()),
                    read: if n.is_multiple_of(3) {
                        ReadState::Unread
                    } else {
                        ReadState::Read
                    },
                    star: Star::Unstarred,
                    mailbox: MailboxRole::Inbox,
                    labels: vec![],
                    body: Body::Present {
                        text: Some(text),
                        raw,
                    },
                    attachments: vec![],
                };
                Fetched {
                    remote: RemoteRef::Pop { uidl: key },
                    key: message.key.clone(),
                    raw,
                    message,
                }
            })
            .collect();
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

/// The rows one page of the list asks for: the window's `PAGE`.
const PAGE: usize = 100;

/// Times each common word is searched for; the medians are compared.
const COMMON_RUNS: usize = 7;

/// What the top results may add to a common word's date-ordered list, as a multiple of it.
const STRIP_OVERHEAD: f64 = 1.5;

/// The store, with the time spent in each of its three calls written down.
struct Timed<'a> {
    store: &'a SqliteStore,
    vocabulary: Cell<Duration>,
    listed: Cell<Duration>,
    top: Cell<Duration>,
}

impl Source for Timed<'_> {
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
        let start = Instant::now();
        let terms = <SqliteStore as Source>::terms_with_prefix(self.store, prefix, limit);
        self.vocabulary.set(self.vocabulary.get() + start.elapsed());
        terms
    }

    fn listed(&self, filter: &Filter, page: PageReq, now: DateTime<Utc>) -> Vec<ThreadSummary> {
        let start = Instant::now();
        let rows = <SqliteStore as Source>::listed(self.store, filter, page, now);
        self.listed.set(self.listed.get() + start.elapsed());
        rows
    }

    fn top(
        &self,
        filter: &Filter,
        k: usize,
        window: usize,
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        let start = Instant::now();
        let rows = <SqliteStore as Source>::top(self.store, filter, k, window, now);
        self.top.set(self.top.get() + start.elapsed());
        rows
    }
}

/// One keystroke's cost: in total, and in each store call.
struct Cost {
    total: Duration,
    vocabulary: Duration,
    listed: Duration,
    top: Duration,
    best: Option<String>,
    newest: Option<DateTime<Utc>>,
}

/// One keystroke: the typed text through the list box's whole pipeline, one page and the strip.
fn keystroke(store: &SqliteStore, typed: &str, now: DateTime<Utc>) -> Cost {
    let timed = Timed {
        store,
        vocabulary: Cell::new(Duration::ZERO),
        listed: Cell::new(Duration::ZERO),
        top: Cell::new(Duration::ZERO),
    };
    let start = Instant::now();
    let searched = search_list(
        typed,
        &timed,
        &Affinity::default(),
        &Utc,
        &|_| Vec::new(),
        first(PAGE),
        now,
    )
    .unwrap_or_else(|why| panic!("{typed:?} is plain text, not a pattern: {why}"));
    let total = start.elapsed();
    Cost {
        total,
        vocabulary: timed.vocabulary.get(),
        listed: timed.listed.get(),
        top: timed.top.get(),
        best: searched
            .top
            .into_iter()
            .next()
            .map(|(summary, _)| summary.subject),
        newest: searched.rows.first().map(|summary| summary.last_date),
    }
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    let index = (sorted.len() * p).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    percentile(&samples, 50)
}

// Ignored in every build and run explicitly in release, as the module docs say. In a debug
// build the same pipeline, SQLite included, is compiled without optimisation, so the number
// would measure the build and not the search; and ingesting fifty thousand messages there
// takes minutes of every `cargo test`.
#[test]
#[ignore = "50k-message scale test: run with --release -- --ignored --nocapture"]
fn typing_uidvalidity_stays_under_thirty_ms_a_keystroke() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let started = Instant::now();
    fill(&store);
    eprintln!("ingested {MESSAGES} messages in {:?}", started.elapsed());
    let now = at(MESSAGES + 60);

    // Not vacuous: the whole word ranks a thread with it in the subject first, and lists the
    // newest thread about it at the top of the date order.
    let whole = keystroke(&store, TYPED, now);
    let best = whole.best.unwrap_or_default();
    assert!(
        best.split_whitespace()
            .any(|word| word.eq_ignore_ascii_case(TYPED)),
        "the typed word did not put a thread about it first: {best:?}"
    );
    let newest_about = at((MESSAGES - 1) / ABOUT_UIDVALIDITY * ABOUT_UIDVALIDITY);
    assert!(
        whole.newest.is_some_and(|newest| newest >= newest_about),
        "the list does not start at the newest match: {:?}, want at least {newest_about}",
        whole.newest
    );

    // One untimed pass, so the first sample is not also SQLite paging the index in.
    for end in 1..=TYPED.len() {
        keystroke(&store, &TYPED[..end], now);
    }

    let mut samples = Vec::with_capacity(ROUNDS * TYPED.len());
    let mut worst: Vec<Option<Cost>> = (0..TYPED.len()).map(|_| None).collect();
    for _ in 0..ROUNDS {
        for end in 1..=TYPED.len() {
            let cost = keystroke(&store, &TYPED[..end], now);
            samples.push(cost.total);
            let slot = &mut worst[end - 1];
            if slot.as_ref().is_none_or(|seen| cost.total > seen.total) {
                *slot = Some(cost);
            }
        }
    }
    samples.sort();
    let p50 = percentile(&samples, 50);
    let p95 = percentile(&samples, 95);
    let max = samples.last().copied().unwrap_or_default();
    eprintln!(
        "{} keystrokes over {MESSAGES} messages: p50 {p50:?}, p95 {p95:?}, max {max:?}",
        samples.len()
    );
    eprintln!("  typed        worst       vocabulary  listed      top");
    for (end, cost) in worst.iter().enumerate() {
        if let Some(cost) = cost {
            eprintln!(
                "  {:<12} {:<11?} {:<11?} {:<11?} {:?}",
                &TYPED[..=end],
                cost.total,
                cost.vocabulary,
                cost.listed,
                cost.top
            );
        }
    }

    // A whole word in a large share of the mailbox. Its absolute time is `Store::threads`'s
    // own, one indexed page of a word nearly every message has, and the debounce hides it, so
    // it is reported and not asserted. What is asserted is that the strip does not bring back
    // the cost of ranking every match: the window bounds `top`, whatever the word.
    let mut overheads = Vec::new();
    for word in ["the ", "us ", "update "] {
        keystroke(&store, word, now);
        let costs: Vec<Cost> = (0..COMMON_RUNS)
            .map(|_| keystroke(&store, word, now))
            .collect();
        let listed = median(costs.iter().map(|cost| cost.listed).collect());
        let top = median(costs.iter().map(|cost| cost.top).collect());
        let total = median(costs.iter().map(|cost| cost.total).collect());
        let ratio = (listed + top).as_secs_f64() / listed.as_secs_f64().max(f64::EPSILON);
        eprintln!(
            "  whole word {:<8} total {:<11?} listed {:<11?} top {:<11?} listed+top {ratio:.2}x listed",
            word.trim(),
            total,
            listed,
            top
        );
        overheads.push((word.trim(), ratio));
    }

    assert!(
        p95 < BUDGET_P95,
        "p95 {p95:?} is over the {BUDGET_P95:?} budget (p50 {p50:?}, max {max:?})"
    );
    for (word, ratio) in overheads {
        assert!(
            ratio <= STRIP_OVERHEAD,
            "{word:?}: listed + top is {ratio:.2}x listed alone, over {STRIP_OVERHEAD}x"
        );
    }
}
