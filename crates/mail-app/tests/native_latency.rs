//! How long a key takes to reach the screen in the real window on Blitz — `plan.md` phase 8, the
//! half `frame_budget.rs` cannot see.
//!
//! `frame_budget` times what the store and the reader do on the render thread. This times what a
//! person waits for: from the key going in to the frame that shows its answer painted, through
//! the components, the off-thread reads, style, layout and paint (`ds_harness::Harness`). `j`,
//! `k`, `e`, opening a row, a search, and the window's cold start to its first painted list.
//!
//! The harness runs on [`Clock::Virtual`], so a timer or an animation takes no wall time; what is
//! timed is the machine's work, plus the real time the off-thread reads take. Paint is vello_cpu
//! at the window's size, which is slower than the GPU the window paints with, so it is printed on
//! its own. A search includes the box's quiet period (`debounce::QUIET`, 150 ms), which runs on
//! real time.
//!
//! Ignored by default, and it prints rather than asserts, for the reason `frame_budget` gives.
//! Run it with
//!
//! ```text
//! cargo test -p mail-app --test native_latency -- --ignored --nocapture
//! ```

#[path = "support/drive.rs"]
mod drive;
#[path = "support/settle.rs"]
mod settle;

use drive::{Drive, Key};
use ds::prelude::{Point, Px};
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};
use mail_app::ui::appearance::WindowDirs;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use settle::{WAIT_BOUND, settle_until};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c3"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// Conversations in the inbox: more than a page, so the list is as long as it gets.
const THREADS: usize = 300;

/// Presses of each key timed.
const PRESSES: usize = 20;

/// What the subjects are about; a search for one of these finds a fifth of the inbox.
const TOPICS: [&str; 5] = ["invoices", "travel", "hiring", "roadmap", "lunches"];

/// An HTML newsletter-sized body: a few kilobytes of markup, the shape that costs a render.
fn raw(n: usize, date: &str) -> Vec<u8> {
    let topic = TOPICS[n % TOPICS.len()];
    let paragraphs = format!("<p>About {topic}, item {n}, and what comes next.</p>").repeat(40);
    format!(
        "From: sender{n}@example.test\r\nTo: me@example.test\r\n\
         Subject: Note {n} about {topic}\r\nDate: {date}\r\n\
         Message-ID: <latency{n}@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         <html><body><h1>Note {n}</h1>{paragraphs}</body></html>\r\n"
    )
    .into_bytes()
}

/// A store in `dir` with one account and [`THREADS`] conversations of one message each.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    let arrivals = (0..THREADS)
        .map(|n| {
            let date = (now - chrono::Duration::minutes(n as i64 + 1)).to_rfc2822();
            Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("latency{n}"),
                },
                raw: raw(n, &date),
            }
        })
        .collect();
    absorb(
        &store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        arrivals,
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

fn config(store: &Arc<SqliteStore>, dir: &std::path::Path) -> HarnessConfig {
    let dirs = WindowDirs {
        config: dir.join("config"),
        state: dir.join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        Some(dirs),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts)
}

fn listed(harness: &Harness) -> bool {
    harness.count(".list .ds-thread") > 0
}

fn subject_of_open(harness: &Harness) -> Option<String> {
    harness.text_of(".reader-head h2")
}

/// What one input cost: until the document showed its answer, then the paint of that frame.
#[derive(Clone, Copy)]
struct Took {
    drawn: Duration,
    paint: Duration,
}

/// Send `act`, step the harness until `done`, paint, and say how long each part took.
fn time(
    harness: &mut Harness,
    act: impl FnOnce(&mut Harness),
    done: impl Fn(&Harness) -> bool,
) -> Took {
    let start = Instant::now();
    act(harness);
    // The harness's clock moves by the real time each step took, so an animation or a timer
    // takes as long as it would in the window, and no longer.
    let mut stepped = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < WAIT_BOUND,
            "never answered:\n{}",
            harness.html()
        );
        let now = Instant::now();
        harness.advance((now - stepped).max(Duration::from_millis(1)));
        stepped = now;
        std::thread::yield_now();
    }
    let drawn = start.elapsed();
    let paint = harness.paint_timed().expect("vello_cpu paints").total;
    Took { drawn, paint }
}

fn report(what: &str, mut took: Vec<Took>) {
    let pick = |took: &mut Vec<Took>, by: fn(&Took) -> Duration| {
        took.sort_by_key(by);
        let at = |q: f64| by(&took[((took.len() - 1) as f64 * q).round() as usize]);
        (at(0.5), at(0.95), at(1.0))
    };
    let (d50, d95, dmax) = pick(&mut took, |t| t.drawn);
    let (p50, p95, _) = pick(&mut took, |t| t.paint);
    println!(
        "{what:<34} drawn p50 {:>7.2} ms  p95 {:>7.2} ms  max {:>7.2} ms   paint p50 {:>6.2} ms  p95 {:>6.2} ms",
        ms(d50),
        ms(d95),
        ms(dmax),
        ms(p50),
        ms(p95),
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The `n`th item of the list (1-based).
fn item(n: usize) -> String {
    format!(".list .ds-list > .ds-list-item:nth-child({n})")
}

/// The `n`th row of the list (1-based).
fn row(n: usize) -> String {
    format!("{} .ds-row", item(n))
}

/// Where a person clicks the `n`th row: the start of its subject line.
fn row_subject(harness: &Harness, n: usize) -> Point {
    let selector = format!("{} .ds-thread-sub", row(n));
    let rect = harness
        .rect(&selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    }
}

/// A key that moves the open conversation: done once the reader names another one.
fn moving(harness: &mut Harness, key: Key) -> Took {
    let was = subject_of_open(harness);
    time(
        harness,
        |h| h.key(key),
        |h| subject_of_open(h).is_some_and(|now| Some(now) != was),
    )
}

#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn from_a_key_to_its_frame() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = Harness::new(mail_app::ui::native::root, config(&store, dir.path()));
    settle_until(&mut harness, listed);
    println!(
        "{THREADS} conversations of a few kilobytes of HTML each, {}x{}",
        VIEW.width, VIEW.height
    );

    // Each click a row not open yet, among those the window shows without scrolling.
    let mut opened = Vec::new();
    for n in 2..=8 {
        let want = harness
            .text_of(&format!("{} .ds-thread-sub", row(n)))
            .unwrap_or_else(|| panic!("row {n} is not drawn"));
        let at = row_subject(&harness, n);
        opened.push(time(
            &mut harness,
            |h| h.click(at),
            |h| subject_of_open(h).is_some_and(|now| now.trim() == want.trim()),
        ));
    }
    report("click a row to its conversation", opened);

    // From the top: `j` walks down into conversations nothing has opened yet.
    let at = row_subject(&harness, 1);
    harness.click(at);
    settle_until(&mut harness, |h| subject_of_open(h).is_some());
    let next: Vec<Took> = (0..PRESSES)
        .map(|_| moving(&mut harness, Key::Char('j')))
        .collect();
    report("j to the next conversation", next);
    let previous: Vec<Took> = (0..PRESSES)
        .map(|_| moving(&mut harness, Key::Char('k')))
        .collect();
    report("k to the previous conversation", previous);
    // Archiving leaves the conversation open and takes its row out of the list: timed until the
    // row starts to leave (its exit animation is the design's, not a wait). `k` has walked back to the top, so the open conversation is the top row; after
    // each archive `j` opens the new top row (a conversation that left the list steps to the
    // first), so the next `e` archives that. Keys, not clicks: on the virtual clock two clicks at
    // one spot are a double click.
    let first = |h: &Harness| h.text_of(&format!("{} .ds-thread-sub", row(1)));
    let same = |open: Option<String>, row: Option<String>| {
        open.zip(row)
            .is_some_and(|(open, row)| open.trim() == row.trim())
    };
    let archived: Vec<Took> = (0..PRESSES)
        .map(|_| {
            settle_until(&mut harness, |h| same(subject_of_open(h), first(h)));
            let was = first(&harness);
            let took = time(
                &mut harness,
                |h| h.key(Key::Char('e')),
                |h| {
                    first(h) != was
                        || h.attr(&item(1), "data-presence").as_deref() == Some("leaving")
                },
            );
            harness.key(Key::Char('j'));
            took
        })
        .collect();
    report("e until the row leaves the list", archived);

    let mut searched = Vec::new();
    for topic in TOPICS {
        let search = harness.centre(".search input").expect("a search box");
        harness.click(search);
        harness.chord(&[Key::Ctrl], Key::Char('a'));
        let (typed, last) = topic.split_at(topic.len() - 1);
        for key in typed.chars() {
            harness.key(Key::Char(key));
        }
        let last = last.chars().next().unwrap();
        searched.push(time(
            &mut harness,
            |h| h.key(Key::Char(last)),
            |h| {
                // A fifth of the inbox matches; the first and last rows drawn both say the topic.
                let rows = h.count(".list .ds-thread");
                let says = |n: usize| {
                    h.text_of(&format!("{} .ds-thread-sub", row(n)))
                        .is_some_and(|subject| subject.contains(topic))
                };
                rows > 0 && rows <= THREADS / TOPICS.len() && says(1) && says(rows)
            },
        ));
    }
    report("last letter of a search to its rows", searched);
}

#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn from_launch_to_the_first_painted_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let starts: Vec<Took> = (0..5)
        .map(|_| {
            let start = Instant::now();
            let mut harness = Harness::new(mail_app::ui::native::root, config(&store, dir.path()));
            while !listed(&harness) {
                assert!(start.elapsed() < WAIT_BOUND, "the list never came");
                harness.advance(Duration::from_millis(1));
                std::thread::yield_now();
            }
            let drawn = start.elapsed();
            let paint = harness.paint_timed().expect("vello_cpu paints").total;
            Took { drawn, paint }
        })
        .collect();
    report("launch to the first list", starts);
}
