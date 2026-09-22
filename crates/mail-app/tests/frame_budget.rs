//! What one frame of the window costs — `plan.md` phase 8a.
//!
//! Every lever in phase 8 is a guess about where the milliseconds are until this file exists.
//! `CONVENTIONS.md` says scale is measured rather than assumed, and the measurement that matters
//! here is not "how fast is the store" but "what does the shell do on the render thread", which
//! is a different question with a different answer.
//!
//! What the window actually does per render, from `ui::App`: one thread-list query, six badge
//! counts, one label index, one draft list, and — when a conversation is open — a blob read plus
//! a full MIME parse, sanitize and inline-embed for every message in it. All of that runs inside
//! `use_memo` closures, which is to say on the thread that draws, and all of it re-runs on every
//! keystroke in the search box.
//!
//! Ignored by default. It prints rather than asserts a budget: a loaded machine would make a
//! timing assertion flaky, and a flaky performance test is a deleted performance test. Run it
//! with
//!
//! ```text
//! cargo test -p mail-app --test frame_budget -- --ignored --nocapture
//! ```
//!
//! `MAILO_BENCH_DB` points it at a real database instead of a generated one, read-only.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use mail_store::{SqliteStore, Store};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/attach.rs"]
mod attach;
#[allow(dead_code)]
#[path = "../src/query.rs"]
mod query;
#[path = "../src/reader.rs"]
mod reader;
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

/// Messages in the generated store. Larger than the mailbox this was designed around, because
/// the interesting question is what happens to someone who has used it for a year.
const MESSAGES: i64 = 10_000;

/// Messages in the conversation that gets opened.
const THREAD_LENGTH: usize = 40;

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n * 60, 0).unwrap()
}

fn policy() -> SanitizePolicy {
    SanitizePolicy {
        remote_images: RemoteImages::Blocked,
        version: SanitizePolicy::CURRENT.version,
    }
}

/// An HTML message with an inline image — the shape that costs something to render.
fn raw_message(n: i64) -> Vec<u8> {
    let filler = "widgets and quarterly reports. ".repeat(60);
    format!(
        "From: s{n}@example.test\r\n\
         Subject: message {n} about quarterly widgets\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/related; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <html><body><p>{filler}</p><img src=\"cid:pic\"><blockquote>{filler}</blockquote>\
         </body></html>\r\n\
         --b\r\nContent-Type: image/png\r\nContent-ID: <pic>\r\n\
         Content-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n"
    )
    .into_bytes()
}

fn generated() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("bench.db"), dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    // One long conversation, so the reader has something real to open, then the rest as
    // singletons — which is what a mailbox mostly is.
    let long = ThreadId::generate();
    for batch in 0..(MESSAGES / 500) {
        let mut messages = Vec::with_capacity(500);
        for i in 0..500 {
            let n = batch * 500 + i;
            let raw = store
                .blobs()
                .put(&store.connection(), &raw_message(n))
                .unwrap();
            let key = format!("m{n}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread: if (n as usize) < THREAD_LENGTH {
                    long
                } else {
                    ThreadId::generate()
                },
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
    (store, dir)
}

/// The real database, when one is named, or a generated one.
fn subject() -> (SqliteStore, Option<tempfile::TempDir>, String) {
    match std::env::var_os("MAILO_BENCH_DB") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            let blobs = path.parent().unwrap_or(std::path::Path::new(".")).join("blobs");
            let store = SqliteStore::open(&path, blobs).expect("that database opens");
            let count: i64 = store
                .connection()
                .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap_or(0);
            (store, None, format!("{} ({count} messages)", path.display()))
        }
        None => {
            let (store, dir) = generated();
            (store, Some(dir), format!("generated ({MESSAGES} messages)"))
        }
    }
}

fn accounts(store: &SqliteStore) -> Vec<AccountId> {
    let db = store.connection();
    let mut stmt = db.prepare("SELECT id FROM accounts ORDER BY created_at").unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.filter_map(Result::ok)
        .filter_map(|id| id.parse().ok())
        .map(AccountId::from_uuid)
        .collect()
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

/// Run `body` a few times and return the median, which is what a user experiences.
fn timed(times: usize, mut body: impl FnMut()) -> Duration {
    let mut runs: Vec<Duration> = Vec::with_capacity(times);
    for _ in 0..times {
        let start = Instant::now();
        body();
        runs.push(start.elapsed());
    }
    runs.sort();
    runs[runs.len() / 2]
}

#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_one_frame_of_the_window_costs() {
    let (store, _dir, what) = subject();
    let now = Utc::now();
    println!("\n  against {what}\n");

    // 1. The list. One query for the whole visible list, deliberately — see `ui::App`.
    let list = timed(7, || {
        let _ = store.threads(&page(50, view::place_filter(MailboxRole::Inbox)), now);
    });

    // 2. The badges: one count per place, every revision.
    let places = view::default_places();
    let badges = timed(7, || {
        for place in &places {
            if let view::Source::Mail(filter) = &place.source {
                let _ = store.count(filter, now);
            }
        }
    });

    // 3. The label index, re-read on every revision.
    let labels = timed(7, || {
        let _ = query::known_labels(&store);
    });

    // 4. The drafts list.
    let drafts = timed(7, || {
        for account in accounts(&store) {
            let _ = store.drafts(account);
        }
    });

    // 5. The open conversation: a blob read, a full MIME parse, sanitize, and inline embed,
    //    per message, on every render.
    // The longest conversation in the newest few hundred. Deliberately not `Filter::All` over
    // one page: that returns the *newest* 200 by date, and a fixture's long thread may be the
    // oldest — which is how this first reported a one-message reader on a store built to have a
    // forty-message one.
    let mut candidates = store.threads(&page(500, Filter::All), now).unwrap().items;
    candidates.extend(
        store
            .threads(
                &Query {
                    sort: Sort {
                        property: Property::Date,
                        dir: SortDir::Asc,
                    },
                    ..page(500, Filter::All)
                },
                now,
            )
            .unwrap()
            .items,
    );
    let longest = candidates
        .into_iter()
        .max_by_key(|t| t.message_count)
        .expect("the store has mail");
    let loaded = store.thread(longest.id).unwrap();
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let reader = timed(5, || {
        for message in &messages {
            let _ = reader::render(&store, message, policy());
        }
    });

    // 6. A search keystroke: parse plus query, which is what each character costs.
    let index = query::known_labels(&store);
    let search = timed(7, || {
        let filter = query::parse_with("widgets", &chrono::Local, &query::named(&index));
        let _ = store.threads(&page(50, filter), now);
    });

    let ms = |d: Duration| format!("{:>8.2} ms", d.as_secs_f64() * 1000.0);
    println!("  list of 50            {}", ms(list));
    println!("  six badge counts      {}", ms(badges));
    println!("  label index           {}", ms(labels));
    println!("  drafts                {}", ms(drafts));
    println!(
        "  reader ({:>3} messages) {}",
        messages.len(),
        ms(reader)
    );
    println!("  search, per keystroke {}", ms(search));

    // What the render thread actually pays for one character typed with a conversation open.
    let keystroke = list + badges + labels + drafts + reader + search;
    println!("\n  one keystroke, reading  {}", ms(keystroke));
    println!(
        "  frames at 60 Hz         {:>8.1}\n",
        keystroke.as_secs_f64() / (1.0 / 60.0)
    );
}

/// The one thing here that is a check rather than a measurement.
///
/// Not a budget in milliseconds — that would be flaky on a loaded machine and would be deleted
/// the first time CI hiccuped. It is the *shape*: rendering the same message twice must not cost
/// twice, once 8d caches it. Until then this records what is true today, which is that it does.
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn rendering_the_same_message_twice_costs_twice() {
    let (store, _dir, _) = subject();
    let message = store
        .threads(&page(1, Filter::All), Utc::now())
        .unwrap()
        .items
        .first()
        .and_then(|t| store.thread(t.id).ok())
        .and_then(|loaded| loaded.messages.first().copied())
        .and_then(|id| store.message(id).ok())
        .expect("the store has mail");

    let once = timed(5, || {
        let _ = reader::render(&store, &message, policy());
    });
    let twice = timed(5, || {
        let _ = reader::render(&store, &message, policy());
        let _ = reader::render(&store, &message, policy());
    });
    println!(
        "\n  one render {:.2} ms, two renders {:.2} ms\n",
        once.as_secs_f64() * 1000.0,
        twice.as_secs_f64() * 1000.0
    );
}
