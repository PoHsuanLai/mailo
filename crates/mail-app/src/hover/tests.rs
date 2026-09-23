//! The hover timer against a fake clock. Every step names the moment it happens.

use super::{At, CLOSE, OPEN, Timer, WARM};

/// One thing that happens to the timer, at a moment.
#[derive(Debug, Clone, Copy)]
enum Step {
    Enter(&'static str),
    Leave,
    EnterCard,
    LeaveCard,
    Dismiss,
    Tick,
}

/// `(when, what happens, the card showing afterwards, warm afterwards)`.
type Script<'a> = &'a [(u64, Step, Option<&'static str>, bool)];

fn play(name: &str, script: Script<'_>) {
    let mut timer: Timer<&'static str> = Timer::default();
    for (index, &(when, step, want_open, want_warm)) in script.iter().enumerate() {
        let now = At(when);
        match step {
            Step::Enter(key) => timer.enter(key, now),
            Step::Leave => timer.leave(now),
            Step::EnterCard => timer.enter_card(),
            Step::LeaveCard => timer.leave_card(now),
            Step::Dismiss => timer.dismiss(now),
            Step::Tick => timer.tick(now),
        }
        assert_eq!(
            timer.open().copied(),
            want_open,
            "{name}, step {index} ({step:?} at {when} ms): wrong card"
        );
        assert_eq!(
            timer.warm(now),
            want_warm,
            "{name}, step {index} ({step:?} at {when} ms): warm"
        );
    }
}

#[test]
fn a_card_waits_for_the_pointer_to_rest() {
    play(
        "open delay",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN - 1, Step::Tick, None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
        ],
    );
}

#[test]
fn passing_over_a_row_opens_nothing() {
    play(
        "passing",
        &[
            (0, Step::Enter("row-1"), None, false),
            (200, Step::Leave, None, false),
            (OPEN, Step::Tick, None, false),
            (OPEN + 400, Step::Tick, None, false),
        ],
    );
}

#[test]
fn moving_to_the_next_row_restarts_the_rest() {
    play(
        "restart",
        &[
            (0, Step::Enter("row-1"), None, false),
            (300, Step::Enter("row-2"), None, false),
            (OPEN, Step::Tick, None, false),
            (300 + OPEN, Step::Tick, Some("row-2"), false),
        ],
    );
}

#[test]
fn a_closed_card_leaves_the_window_warm_and_the_next_opens_at_once() {
    let closed = OPEN + 10 + CLOSE;
    play(
        "warm skip",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (OPEN + 10, Step::Leave, Some("row-1"), false),
            (closed, Step::Tick, None, true),
            (closed + WARM - 1, Step::Enter("row-2"), Some("row-2"), true),
        ],
    );
}

#[test]
fn warmth_runs_out() {
    let closed = OPEN + 10 + CLOSE;
    play(
        "warm expiry",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (OPEN + 10, Step::Leave, Some("row-1"), false),
            (closed, Step::Tick, None, true),
            (closed + WARM, Step::Enter("row-2"), None, false),
            (closed + WARM + OPEN, Step::Tick, Some("row-2"), false),
        ],
    );
}

#[test]
fn the_card_stays_for_its_grace_and_then_closes() {
    play(
        "close grace",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (1000, Step::Leave, Some("row-1"), false),
            (1000 + CLOSE - 1, Step::Tick, Some("row-1"), false),
            (1000 + CLOSE, Step::Tick, None, true),
        ],
    );
}

#[test]
fn the_pointer_can_travel_into_the_card() {
    play(
        "travel",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (1000, Step::Leave, Some("row-1"), false),
            (1000 + CLOSE - 20, Step::EnterCard, Some("row-1"), false),
            (1000 + CLOSE * 4, Step::Tick, Some("row-1"), false),
            (2000, Step::LeaveCard, Some("row-1"), false),
            (2000 + CLOSE, Step::Tick, None, true),
        ],
    );
}

#[test]
fn coming_back_to_the_hook_in_time_keeps_the_card() {
    play(
        "return",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (1000, Step::Leave, Some("row-1"), false),
            (1100, Step::Enter("row-1"), Some("row-1"), false),
            (1000 + CLOSE, Step::Tick, Some("row-1"), false),
        ],
    );
}

#[test]
fn the_innermost_hook_replaces_the_row_while_a_card_shows() {
    // The sender name inside an open row's card: the caller reports the name, and a showing
    // card makes the window warm, so the sender card replaces the thread card at once.
    play(
        "innermost",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (600, Step::Enter("sender-1"), Some("sender-1"), false),
            (700, Step::Enter("row-1"), Some("row-1"), false),
        ],
    );
}

#[test]
fn dismissing_closes_now_and_warms() {
    play(
        "dismiss",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (500, Step::Dismiss, None, true),
            (500 + WARM, Step::Tick, None, false),
        ],
    );
}

#[test]
fn the_deadline_is_the_next_moment_anything_changes() {
    let mut timer: Timer<&str> = Timer::default();
    assert_eq!(timer.deadline(At(0)), None, "an idle timer sleeps");
    timer.enter("row-1", At(10));
    assert_eq!(timer.deadline(At(10)), Some(At(10 + OPEN)));
    timer.tick(At(10 + OPEN));
    assert_eq!(timer.deadline(At(10 + OPEN)), None, "an open card waits");
    timer.leave(At(1000));
    assert_eq!(timer.deadline(At(1000)), Some(At(1000 + CLOSE)));
    timer.tick(At(1000 + CLOSE));
    assert_eq!(
        timer.deadline(At(1000 + CLOSE)),
        Some(At(1000 + CLOSE + WARM)),
        "warmth has to be cleared on time"
    );
}
