//! Typing into the composer's body on the real window, timed: what each key cost the thread that
//! draws, and how long after it the page's caret stood where the text now ends.
//!
//! The harness runs on the virtual clock, so the caret's lag is read in the window's own time
//! (the clock moves 1 ms per step between keys) and does not depend on how loaded the machine
//! is; only `work`, the wall time `Harness::key` took (the handlers, the renders, style and
//! layout of the frame that shows the letter), does.

#![allow(dead_code)]

use super::drive::{Drive, Key};
use super::settle::settle_until;
use ds::prelude::Rect;
use ds_harness::{Driver, Harness, Query};
use std::time::{Duration, Instant};

/// The caret the page draws over the body.
pub const CARET: &str = ".c-marks .c-caret";

/// What is typed into the body: a sentence of ordinary words, 182 characters.
pub const TYPED: &str = "Thanks for the notes on the roadmap. I read them twice and the plan for the spring \
looks right to me, though the hiring part wants another pass before we share it with everyone else.";

/// Click `line` (a paragraph of the body) and paste six paragraphs, as a message under way
/// holds; the caret is at their end, drawn.
pub fn fill_body(harness: &mut Harness, line: &str) {
    let at = harness
        .centre(line)
        .unwrap_or_else(|| panic!("{line} is not drawn:\n{}", harness.html()));
    harness.click(at);
    let one = "We met on Tuesday to go through the numbers for the quarter, and most of what was \
               planned has landed. The travel budget ran over, the invoices are late again, and \
               lunch on Fridays is now a standing item. ";
    let html = format!("<p>{}</p>", one.repeat(2)).repeat(6);
    let text = format!("{}\n\n", one.repeat(2)).repeat(6);
    harness.paste_html(&html, &text);
    settle_until(harness, |h| h.count(CARET) == 1);
}

/// Where the caret settles once nothing more is typed: half a second of the window's time on.
pub fn settled_caret(harness: &mut Harness) -> Option<Rect> {
    harness.advance(Duration::from_millis(500));
    caret(harness)
}

/// What one typed key cost.
#[derive(Debug, Clone, Copy)]
pub struct Typed {
    /// Wall time of the key: its handlers, the renders it caused, style and layout.
    pub work: Duration,
    /// Virtual milliseconds from the key to the first frame whose caret had moved on from where
    /// it stood when the key was pressed (to this key's place, or a later key's), or `None` when
    /// it never did.
    pub caret: Option<u64>,
}

/// The caret as drawn now.
pub fn caret(harness: &Harness) -> Option<Rect> {
    harness.rect(CARET)
}

/// Type `text` with `interval` of the window's time between keys, timing each key; then wait up
/// to half a second for the caret to catch up with the last ones.
pub fn type_timed(harness: &mut Harness, text: &str, interval: Duration) -> Vec<Typed> {
    let steps = interval.as_millis().max(1) as u64;
    let mut now = 0u64;
    let mut typed: Vec<Typed> = Vec::new();
    // Keys whose caret has not moved yet: their index, when they were pressed, and the caret then.
    let mut waiting: Vec<(usize, u64, Option<Rect>)> = Vec::new();
    let look = |harness: &Harness, now: u64, typed: &mut Vec<Typed>, waiting: &mut Vec<_>| {
        let shown = caret(harness);
        waiting.retain(|(index, at, was): &(usize, u64, Option<Rect>)| {
            if shown == *was {
                return true;
            }
            typed[*index].caret = Some(now - at);
            false
        });
    };
    for c in text.chars() {
        let was = caret(harness);
        let key = if c == ' ' { Key::Space } else { Key::Char(c) };
        let start = Instant::now();
        harness.key(key);
        let work = start.elapsed();
        typed.push(Typed { work, caret: None });
        waiting.push((typed.len() - 1, now, was));
        look(harness, now, &mut typed, &mut waiting);
        for _ in 0..steps {
            harness.advance(Duration::from_millis(1));
            now += 1;
            look(harness, now, &mut typed, &mut waiting);
        }
    }
    for _ in 0..500 {
        if waiting.is_empty() {
            break;
        }
        harness.advance(Duration::from_millis(1));
        now += 1;
        look(harness, now, &mut typed, &mut waiting);
    }
    typed
}

/// Print a run of typing: the work per key, and how late the caret was.
pub fn report_typing(what: &str, mut typed: Vec<Typed>) {
    let late = typed.iter().filter(|t| t.caret.is_none()).count();
    let mut lags: Vec<u64> = typed.iter().filter_map(|t| t.caret).collect();
    lags.sort_unstable();
    typed.sort_by_key(|t| t.work);
    let work = |q: f64| typed[((typed.len() - 1) as f64 * q).round() as usize].work;
    let lag = |q: f64| {
        lags.get(((lags.len().max(1) - 1) as f64 * q).round() as usize)
            .copied()
    };
    println!(
        "{what:<34} work p50 {:>6.2} ms  p95 {:>6.2} ms  max {:>6.2} ms   caret p50 {:?} ms  p95 {:?} ms  never moved {late}/{}",
        ms(work(0.5)),
        ms(work(0.95)),
        ms(work(1.0)),
        lag(0.5),
        lag(0.95),
        typed.len(),
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
