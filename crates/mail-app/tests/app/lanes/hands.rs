//! What a person does to any window (the main one, Settings): wait for what they are looking
//! for, click it, type, press keys. Every wait is for a state, bounded by the machine's time.

use ds::prelude::*;
use ds_harness::{Driver, Harness, Query};
use std::time::{Duration, Instant};

use super::drive::{Drive, Key};
use super::settle::WAIT_BOUND;

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Advance `harness`'s clock a little at a time until `done` holds, letting the store's threads
/// run; fail naming `what` after [`WAIT_BOUND`] of the machine's own time.
pub fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let deadline = Instant::now() + WAIT_BOUND;
    while Instant::now() < deadline {
        if done(harness) {
            return;
        }
        harness.advance(ms(10));
        std::thread::sleep(ms(1));
    }
    assert!(
        done(harness),
        "{what}\n(banner: {:?}; toast: {:?})\n{}",
        harness.text_of(".ds-inline-banner-body"),
        harness.text_of(".ds-toast-body"),
        harness.html()
    );
}

/// The centre of `selector`, which must be drawn.
pub fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// The text of `selector`, or nothing.
pub fn text(harness: &Harness, selector: &str) -> String {
    harness.text_of(selector).unwrap_or_default()
}

/// Click `selector` once it is drawn where it stands.
pub fn click(harness: &mut Harness, selector: &str) {
    let what = format!("{selector} is drawn and takes a click");
    until(harness, &what, |h| {
        h.centre(selector).is_some_and(|at| h.hits(at, selector))
    });
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(100));
}

/// Type `text` a key at a time, a space as the space bar and a newline as Enter.
pub fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(match c {
            ' ' => Key::Space,
            '\n' => Key::Enter,
            c => Key::Char(c),
        });
    }
    harness.advance(ms(100));
}

/// Press `key` `times` times with `held` down.
pub fn press(harness: &mut Harness, held: &[Key], key: Key, times: usize) {
    for _ in 0..times {
        harness.chord(held, key);
    }
    harness.advance(ms(100));
}

/// Pick `name` in the open menu once it stands where it is drawn.
pub fn menu_item(harness: &mut Harness, name: &str) {
    let what = format!("a menu offers {name:?}");
    until(harness, &what, |h| {
        super::look::menu_item_of(h, name).is_some()
    });
    let n = super::look::menu_item_of(harness, name).unwrap_or_default();
    click(harness, &format!(".ds-menu > :nth-child({n})"));
    harness.advance(ms(300));
}

/// Scroll `scroller` with the wheel until `selector` stands inside it, as a person scrolls a
/// long page to what they want. The view is read before it scrolls: Blitz reads a scrolled
/// element's rect as moved up by as far as it has scrolled, though it is drawn where it was.
pub fn scroll_to(harness: &mut Harness, scroller: &str, selector: &str) {
    let what = format!("{selector} and {scroller} are drawn");
    until(harness, &what, |h| {
        h.rect(selector).is_some() && h.rect(scroller).is_some()
    });
    let Some(view) = harness.rect(scroller) else {
        return;
    };
    let (top, bottom) = (view.origin.y.0, view.origin.y.0 + view.size.height.0);
    let at = Point {
        x: Px(view.origin.x.0 + view.size.width.0 / 2.0),
        y: Px(top + 4.0),
    };
    for _ in 0..50 {
        let Some(target) = harness.rect(selector) else {
            return;
        };
        let (from, to) = (target.origin.y.0, target.origin.y.0 + target.size.height.0);
        if from >= top - 0.5 && to <= bottom + 0.5 {
            return;
        }
        // A wheel turned towards the person moves the page up, as on any desktop.
        let step = if to > bottom { -120.0 } else { 120.0 };
        harness.wheel(at, Px(0.0), Px(step));
        harness.advance(ms(20));
    }
}
