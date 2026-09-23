//! The debounce over a fake clock: milliseconds the table names, nothing read from the machine.

use super::{Debounce, QUIET, Settled};
use std::time::Duration;

/// One thing that happens, at a millisecond.
enum Step {
    /// The box now holds this text. `Some` when it settles on the spot.
    Type(u64, &'static str, Option<(u64, &'static str)>),
    /// The timer wakes. `Some` when the text is due, with its generation.
    Wake(u64, Option<(u64, &'static str)>),
    /// A result for this generation came back. Whether it may be drawn.
    Landed(u64, bool),
}

fn ms(at: u64) -> Duration {
    Duration::from_millis(at)
}

fn settled(want: Option<(u64, &str)>) -> Option<Settled> {
    want.map(|(generation, text)| Settled {
        generation,
        text: text.to_owned(),
    })
}

#[test]
fn a_query_settles_once_the_box_is_still_and_older_results_are_dropped() {
    use Step::*;
    let cases: &[(&str, &[Step])] = &[
        (
            "one word typed quickly settles once, 150 ms after its last letter",
            &[
                Type(0, "u", None),
                Type(60, "ui", None),
                Wake(150, None),
                Type(160, "uid", None),
                Wake(309, None),
                Wake(310, Some((3, "uid"))),
                Wake(500, None),
            ],
        ),
        (
            "a pause between words searches the first word, and the second supersedes it",
            &[
                Type(0, "lunch", None),
                Wake(150, Some((1, "lunch"))),
                Type(200, "lunch f", None),
                Landed(1, false),
                Wake(350, Some((2, "lunch f"))),
                Landed(2, true),
            ],
        ),
        (
            "a result for the generation still being typed is drawn",
            &[
                Type(0, "cursor", None),
                Wake(150, Some((1, "cursor"))),
                Landed(1, true),
            ],
        ),
        (
            "clearing the box settles on the spot, and nothing is left to wake for",
            &[
                Type(0, "cursor", None),
                Type(50, "", Some((2, ""))),
                Wake(400, None),
                Landed(1, false),
                Landed(2, true),
            ],
        ),
        (
            "the same text again is not a keystroke",
            &[
                Type(0, "a", None),
                Wake(150, Some((1, "a"))),
                Type(400, "a", None),
                Wake(700, None),
                Landed(1, true),
            ],
        ),
        (
            "typing back to the searched text is still a new query",
            &[
                Type(0, "ab", None),
                Wake(150, Some((1, "ab"))),
                Type(200, "abc", None),
                Type(220, "ab", None),
                Landed(1, false),
                Wake(370, Some((3, "ab"))),
            ],
        ),
    ];
    for (name, steps) in cases {
        let mut debounce = Debounce::new(QUIET, "");
        for (at, step) in steps.iter().enumerate() {
            match step {
                Type(when, text, want) => {
                    assert_eq!(
                        debounce.saw(text, ms(*when)),
                        settled(*want),
                        "{name}: step {at}, typed {text:?}"
                    );
                }
                Wake(when, want) => {
                    assert_eq!(
                        debounce.due(ms(*when)),
                        settled(*want),
                        "{name}: step {at}, woke at {when} ms"
                    );
                }
                Landed(generation, drawn) => {
                    assert_eq!(
                        debounce.is_latest(*generation),
                        *drawn,
                        "{name}: step {at}, result of generation {generation}"
                    );
                }
            }
        }
    }
}

/// A name, the keystrokes as `(ms, text)`, when the timer asks, and how long it should sleep.
type Sleep = (&'static str, &'static [(u64, &'static str)], u64, u64);

#[test]
fn the_timer_sleeps_until_the_quiet_period_ends() {
    let cases: &[Sleep] = &[
        ("nothing typed sleeps one quiet period", &[], 0, 150),
        (
            "just typed sleeps the whole quiet period",
            &[(100, "a")],
            100,
            150,
        ),
        ("part way sleeps the rest", &[(100, "a")], 160, 90),
        ("overdue does not sleep", &[(100, "a")], 400, 0),
        (
            "the last keystroke counts",
            &[(0, "a"), (100, "ab")],
            120,
            130,
        ),
    ];
    for (name, typed, now, want) in cases {
        let mut debounce = Debounce::new(QUIET, "");
        for (when, text) in *typed {
            debounce.saw(text, ms(*when));
        }
        assert_eq!(debounce.wait(ms(*now)), ms(*want), "{name}");
    }
}

#[test]
fn the_box_opens_settled_on_what_it_holds() {
    let debounce = Debounce::new(QUIET, "from:dana");
    assert_eq!(
        debounce.settled(),
        Settled {
            generation: 0,
            text: "from:dana".to_owned(),
        }
    );
    assert!(debounce.is_latest(0));
    assert_eq!(debounce.clone().due(ms(10_000)), None, "nothing to search");
}
