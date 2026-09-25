//! The hover rules mailo relies on, against quire's intent machine and a fake clock. These
//! were mailo's own timer's tests; the timer is gone and the hub runs quire's machine, so the
//! same scripts are played against it. Every step names the moment it happens.

use ds::{HoverEvent, HoverIntent, HoverWarmth, IntentEffect, IntentPhase};
use std::time::{Duration, Instant};

/// Rest before a card opens, in milliseconds.
const OPEN: u64 = 450;
/// Grace after the pointer leaves, in which it may reach the card.
const CLOSE: u64 = 150;
/// How long the window stays warm after a card closes.
const WARM: u64 = 400;

/// One thing that happens to the machine, at a moment.
#[derive(Debug, Clone, Copy)]
enum Step {
    Enter(&'static str),
    Leave,
    EnterCard,
    LeaveCard,
    /// A press in the list, or Escape: the card goes at once.
    Press,
    /// Time passes: whichever timer is due fires, as the hub's would.
    Tick,
}

/// `(when, what happens, the card showing afterwards, warm after a close afterwards)`.
type Script<'a> = &'a [(u64, Step, Option<&'static str>, bool)];

struct Clock {
    epoch: Instant,
}

impl Clock {
    fn at(&self, ms: u64) -> Instant {
        self.epoch + Duration::from_millis(ms)
    }
}

/// Feed `event` at `now`, and the open the hub starts at once when the window is warm.
fn feed(
    intent: HoverIntent<&'static str>,
    event: HoverEvent<&'static str>,
    now: Instant,
) -> HoverIntent<&'static str> {
    let (next, effect) = intent.step(event, now);
    match effect {
        IntentEffect::StartOpen { after } if after.is_zero() => {
            next.step(HoverEvent::OpenDue, now).0
        }
        _ => next,
    }
}

fn open(intent: &HoverIntent<&'static str>) -> Option<&'static str> {
    match intent.phase() {
        IntentPhase::Open { key } | IntentPhase::Closing { key, .. } => Some(*key),
        IntentPhase::Idle | IntentPhase::Pending { .. } => None,
    }
}

/// Warm with no card showing: a card closed less than [`WARM`] ago.
fn lingering(intent: &HoverIntent<&'static str>, now: Instant) -> bool {
    open(intent).is_none() && intent.warmth(now) == HoverWarmth::Warm
}

fn play(name: &str, script: Script<'_>) {
    let clock = Clock {
        epoch: Instant::now(),
    };
    let mut intent: HoverIntent<&'static str> = HoverIntent::default();
    for (index, &(when, step, want_open, want_warm)) in script.iter().enumerate() {
        let now = clock.at(when);
        intent = match step {
            Step::Enter(key) => feed(intent, HoverEvent::Over(key), now),
            Step::Leave => feed(intent, HoverEvent::Out, now),
            Step::EnterCard => feed(intent, HoverEvent::EnterCard, now),
            Step::LeaveCard => feed(intent, HoverEvent::LeaveCard, now),
            Step::Press => feed(intent, HoverEvent::ClickInList, now),
            Step::Tick => {
                let opened = feed(intent, HoverEvent::OpenDue, now);
                feed(opened, HoverEvent::CloseDue, now)
            }
        };
        assert_eq!(
            open(&intent),
            want_open,
            "{name}, step {index} ({step:?} at {when} ms): wrong card"
        );
        assert_eq!(
            lingering(&intent, now),
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
            (
                closed + WARM - 1,
                Step::Enter("row-2"),
                Some("row-2"),
                false,
            ),
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
fn a_press_closes_now_and_leaves_the_window_cold() {
    // quire's rule (design/06 section 3): a press in the list removes the card at once and
    // does not warm the window, so the next card waits for rest again. mailo's own timer
    // warmed it; quire's hub is the one the window now runs.
    play(
        "press",
        &[
            (0, Step::Enter("row-1"), None, false),
            (OPEN, Step::Tick, Some("row-1"), false),
            (500, Step::Press, None, false),
            (510, Step::Enter("row-2"), None, false),
            (510 + OPEN, Step::Tick, Some("row-2"), false),
        ],
    );
}

#[test]
fn each_timer_is_due_when_the_rules_say() {
    let clock = Clock {
        epoch: Instant::now(),
    };
    let intent: HoverIntent<&'static str> = HoverIntent::default();
    let intent = feed(intent, HoverEvent::Over("row-1"), clock.at(10));
    assert_eq!(
        intent.phase(),
        &IntentPhase::Pending {
            key: "row-1",
            due: clock.at(10 + OPEN)
        },
        "the card is due after the rest"
    );
    let intent = feed(intent, HoverEvent::OpenDue, clock.at(10 + OPEN));
    assert_eq!(intent.phase(), &IntentPhase::Open { key: "row-1" });
    let intent = feed(intent, HoverEvent::Out, clock.at(1000));
    assert_eq!(
        intent.phase(),
        &IntentPhase::Closing {
            key: "row-1",
            due: clock.at(1000 + CLOSE)
        },
        "the close is due after the grace"
    );
    let intent = feed(intent, HoverEvent::CloseDue, clock.at(1000 + CLOSE));
    let closed = 1000 + CLOSE;
    assert!(lingering(&intent, clock.at(closed + WARM - 1)));
    assert!(
        !lingering(&intent, clock.at(closed + WARM)),
        "warmth has to run out on time"
    );
}
