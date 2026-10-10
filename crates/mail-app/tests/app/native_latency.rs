//! How long a key takes to reach the screen in the real window on Blitz — `plan.md` phase 8, the
//! half `frame_budget.rs` cannot see.
//!
//! `frame_budget` times what the store and the reader do on the render thread. This times what a
//! person waits for: from the key going in to the frame that shows its answer painted, through
//! the components, the off-thread reads, style, layout and paint (`ds_harness::Harness`). `j`,
//! `k`, `e`, opening a row, a search, the window's cold start to its first painted list, and
//! letters typed into the composer's body, to the frame whose caret has moved.
//!
//! The harness runs on `Clock::Virtual`, so a timer or an animation takes no wall time; what is
//! timed is the machine's work, plus the real time the off-thread reads take. Paint is vello_cpu
//! at the window's size, which is slower than the GPU the window paints with, so it is printed on
//! its own. A search includes the box's quiet period (`debounce::QUIET`, 150 ms), which runs on
//! real time.
//!
//! The measurements are ignored by default, and print rather than assert, for the reason
//! `frame_budget` gives. Two checks over the same inbox always run: that the list is windowed,
//! mounting only the rows near the viewport and asking for the next page as its end comes near,
//! and that the composer's caret keeps up with keys closer together than two frames, counted on
//! the virtual clock rather than in milliseconds of the machine's. Run the measurements with
//!
//! ```text
//! cargo test -p mail-app --test app -- --ignored --nocapture native_latency::
//! ```

use crate::drive;
use crate::latency_inbox as inbox;
use crate::settle;
use crate::typing;

use drive::{Drive, Key, PRIMARY};
use ds::prelude::{Point, Px};
use ds_harness::{Driver, Harness, Query as Read};
use inbox::{THREADS, TOPICS, VIEW, config, seeded};
use settle::{WAIT_BOUND, settle_until};
use std::time::{Duration, Instant};
use typing::{TYPED, caret, fill_body, report_typing, settled_caret, type_timed};

/// Presses of each key timed.
const PRESSES: usize = 20;

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
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
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
        // ⌘K: the search panel, its text selected, so the topic replaces the last one.
        harness.chord(&[PRIMARY], Key::Char('k'));
        settle_until(&mut harness, |h| h.is_focused(".ds-search-card input"));
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

/// How many rows the list says it holds: every mounted item carries the set's size.
fn set_size(harness: &Harness) -> Option<usize> {
    harness
        .attr(".list .ds-list-item[*|aria-setsize]", "aria-setsize")
        .and_then(|size| size.parse().ok())
}

#[test]
fn a_long_inbox_mounts_the_rows_near_the_viewport_and_pages_as_its_end_comes_near() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = Harness::new(mail_app::ui::native::root, config(&store, dir.path()));
    settle_until(&mut harness, listed);
    // A page of a hundred is listed; only a window of it is drawn: what an 800 px window shows of
    // 66 px rows, and a few more on each side.
    settle_until(&mut harness, |h| set_size(h) == Some(100));
    let mounted = harness.count(".list .ds-thread");
    assert!(
        (8..=30).contains(&mounted),
        "{mounted} of 100 rows are mounted"
    );
    // Scrolled down towards the end of the page, the next page is asked for and drawn: there is
    // no "Show more" to press.
    let over = harness.centre(".list").expect("the list is drawn");
    let deadline = Instant::now() + WAIT_BOUND;
    while set_size(&harness) != Some(200) {
        assert!(
            Instant::now() < deadline,
            "the next page never came: {:?} rows listed",
            set_size(&harness)
        );
        // A negative delta scrolls down, as a wheel turned towards the person does.
        harness.wheel(over, Px(0.0), Px(-120.0));
        harness.advance(Duration::from_millis(20));
        std::thread::yield_now();
    }
    let mounted = harness.count(".list .ds-thread");
    assert!(
        (8..=30).contains(&mounted),
        "{mounted} of 200 rows are mounted"
    );
}

/// The window over the inbox, with a new message open and six paragraphs in its body.
fn new_message(dir: &std::path::Path) -> Harness {
    let store = seeded(dir);
    let mut harness = Harness::new(mail_app::ui::native::root, config(&store, dir));
    settle_until(&mut harness, listed);
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    fill_body(&mut harness, ".cpage .c-body > p");
    harness
}

#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn from_a_typed_letter_to_its_caret() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = new_message(dir.path());
    for interval in [12, 40, 120] {
        let typed = type_timed(&mut harness, TYPED, Duration::from_millis(interval));
        report_typing(&format!("new message, a key every {interval} ms"), typed);
    }
    harness.key(Key::Escape);
    harness.advance(Duration::from_millis(400));

    // A reply to a long HTML message: its original quoted under the body.
    let at = row_subject(&harness, 1);
    harness.click(at);
    settle_until(&mut harness, |h| subject_of_open(h).is_some());
    harness.key(Key::Char('r'));
    settle_until(&mut harness, |h| h.count(".inline-reply .c-body") == 1);
    fill_body(&mut harness, ".inline-reply .c-body > p");
    for interval in [12, 40] {
        let typed = type_timed(&mut harness, TYPED, Duration::from_millis(interval));
        report_typing(&format!("reply, a key every {interval} ms"), typed);
    }
}

/// Keys a held key repeats, or a fast typist's burst: closer together than two frames.
const BURST: Duration = Duration::from_millis(12);

/// The caret keeps up with typing. Read on the window's own (virtual) clock, so a slow machine
/// changes how long this takes, never what it sees: every key's caret moves within two frames of
/// it, however close the keys come, and two frames after the last key the caret is where it
/// settles. The caret used to wait 34 ms and drop its read whenever a key came in that time, so
/// a burst left it where typing began until the keys stopped.
#[test]
fn the_caret_keeps_up_with_a_burst_of_keys() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = new_message(dir.path());
    let typed = type_timed(&mut harness, &TYPED[..60], BURST);
    let lags: Vec<Option<u64>> = typed.iter().map(|t| t.caret).collect();
    assert!(
        lags.iter().all(|lag| lag.is_some_and(|ms| ms <= 34)),
        "the caret fell behind the keys (ms from each key to its caret): {lags:?}"
    );
    // The last key's caret, two frames on, is the caret once all is still.
    harness.key(Key::Char('x'));
    harness.advance(Duration::from_millis(34));
    let shown = caret(&harness);
    assert_eq!(
        shown,
        settled_caret(&mut harness),
        "the caret moved after it was drawn"
    );
    // Generous: a key's own work in a debug build on a slow runner is a few milliseconds.
    let slowest = typed.iter().map(|t| t.work).max().unwrap_or_default();
    assert!(
        slowest < Duration::from_millis(500),
        "a key took {slowest:?} of the thread that draws"
    );
}
