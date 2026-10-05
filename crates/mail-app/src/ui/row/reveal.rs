//! When a row's hover strip may be pressed.
//!
//! Quire's strip is live the instant `:hover` matches the row, so a pointer that is simply
//! resting where the strip appears (the row's middle-right) clicks Archive or Trash instead of
//! opening the thread. The strip is therefore held down until the pointer has dwelled on the
//! row for [`DWELL`]. The rule is pure: the component feeds it what the pointer did and reads
//! back whether the strip is armed; the timer that says "dwelled" lives in the component.

use ds::prelude::Shown;
use std::time::Duration;

/// How long the pointer must rest on a row before its strip can be pressed.
pub(super) const DWELL: Duration = Duration::from_millis(250);

/// Whether the pointer's rest on a row has lasted long enough to show the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Reveal {
    /// The pointer is away, or arrived too recently: the strip stays down.
    #[default]
    Idle,
    /// The pointer has dwelled: the strip is shown and pressable.
    Armed,
}

/// What the pointer did to the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pointer {
    /// It came onto the row; the dwell starts over.
    Entered,
    /// It rested for [`DWELL`] since the last `Entered`.
    Dwelled,
    /// It went off the row.
    Left,
}

/// Whether the keyboard focus is inside the row, which shows the strip with no pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Within,
    Outside,
}

/// The state after `pointer` happened in state `from`.
pub(super) fn next(from: Reveal, pointer: Pointer) -> Reveal {
    match (from, pointer) {
        (_, Pointer::Entered | Pointer::Left) => Reveal::Idle,
        (_, Pointer::Dwelled) => Reveal::Armed,
    }
}

/// How the strip is drawn: shown for a focused row or an armed pointer, held down otherwise
/// (quire's `data-shown=hidden` keeps it down even under `:hover`).
pub(super) fn shown(reveal: Reveal, focus: Focus) -> Shown {
    match (reveal, focus) {
        (Reveal::Armed, _) | (_, Focus::Within) => Shown::Visible,
        (Reveal::Idle, Focus::Outside) => Shown::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_drives_the_reveal() {
        const CASES: &[(&str, Reveal, Pointer, Reveal)] = &[
            (
                "entering starts unarmed",
                Reveal::Idle,
                Pointer::Entered,
                Reveal::Idle,
            ),
            (
                "re-entering disarms",
                Reveal::Armed,
                Pointer::Entered,
                Reveal::Idle,
            ),
            (
                "dwelling arms",
                Reveal::Idle,
                Pointer::Dwelled,
                Reveal::Armed,
            ),
            (
                "dwelling again stays armed",
                Reveal::Armed,
                Pointer::Dwelled,
                Reveal::Armed,
            ),
            (
                "leaving disarms",
                Reveal::Armed,
                Pointer::Left,
                Reveal::Idle,
            ),
            (
                "leaving while idle",
                Reveal::Idle,
                Pointer::Left,
                Reveal::Idle,
            ),
        ];
        for (name, from, pointer, want) in CASES {
            assert_eq!(next(*from, *pointer), *want, "{name}");
        }
    }

    #[test]
    fn the_strip_is_shown_only_when_armed_or_focused() {
        const CASES: &[(&str, Reveal, Focus, Shown)] = &[
            ("idle and away", Reveal::Idle, Focus::Outside, Shown::Hidden),
            ("armed", Reveal::Armed, Focus::Outside, Shown::Visible),
            ("focused", Reveal::Idle, Focus::Within, Shown::Visible),
            (
                "armed and focused",
                Reveal::Armed,
                Focus::Within,
                Shown::Visible,
            ),
        ];
        for (name, reveal, focus, want) in CASES {
            assert_eq!(shown(*reveal, *focus), *want, "{name}");
        }
    }
}
