//! Deleting a Space, as data: which Space goes, and what the list's indices become.
//!
//! Spaces are keyed by position (`current`, `recall`, and Today's entries), so removing one
//! renumbers every Space after it. [`remove`] does that to the Spaces; `Today::drop_space`
//! does it to Today. Neither touches mail: a Space is a look, some pins and an account scope.

use super::{Recall, Space, Spaces};
use std::collections::BTreeMap;

/// Why a Space was not removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The only Space left: the window always has one.
    Last,
    /// No Space at that index.
    Missing,
}

/// Whether the Space on screen changed with the removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Showing {
    /// The Space on screen is the one still showing, now at [`Removed::now_current`].
    Same,
    /// The Space on screen was removed; [`Removed::now_current`] is another Space.
    Other,
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq)]
pub struct Removed {
    /// The Space that went, whole, so a caller can put it back.
    pub space: Space,
    /// Where it was.
    pub index: usize,
    /// The index of the Space on screen after the removal.
    pub now_current: usize,
    /// Whether that is the Space that was on screen before.
    pub showing: Showing,
}

/// Remove the Space at `index`, renumbering `current` and `recall` to match.
///
/// Refused for the last Space, and for an index past the end; nothing changes then.
pub fn remove(spaces: &mut Spaces, index: usize) -> Result<Removed, Refused> {
    if index >= spaces.spaces.len() {
        return Err(Refused::Missing);
    }
    if spaces.spaces.len() <= 1 {
        return Err(Refused::Last);
    }
    let space = spaces.spaces.remove(index);
    let before = spaces.current;
    let (now_current, showing) = if before == index {
        (index.min(spaces.spaces.len() - 1), Showing::Other)
    } else if index < before {
        (before - 1, Showing::Same)
    } else {
        (before, Showing::Same)
    };
    spaces.current = now_current;
    let old: BTreeMap<usize, Recall> = std::mem::take(&mut spaces.recall);
    spaces.recall = old
        .into_iter()
        .filter(|(key, _)| *key != index)
        .map(|(key, recall)| (if key > index { key - 1 } else { key }, recall))
        .collect();
    Ok(Removed {
        space,
        index,
        now_current,
        showing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaces(count: usize, current: usize) -> Spaces {
        Spaces {
            spaces: (0..count)
                .map(|n| Space {
                    name: format!("S{n}"),
                    ..Space::default()
                })
                .collect(),
            current,
            recall: (0..count)
                .map(|n| {
                    (
                        n,
                        Recall {
                            place: format!("P{n}"),
                            ..Recall::default()
                        },
                    )
                })
                .collect(),
        }
    }

    fn names(spaces: &Spaces) -> Vec<&str> {
        spaces.spaces.iter().map(|s| s.name.as_str()).collect()
    }

    fn places(spaces: &Spaces) -> Vec<(usize, &str)> {
        spaces
            .recall
            .iter()
            .map(|(k, r)| (*k, r.place.as_str()))
            .collect()
    }

    #[test]
    fn removing_renumbers_current_and_recall() {
        // (count, current, index, names left, current after, showing, recall left)
        type Case = (
            usize,
            usize,
            usize,
            &'static [&'static str],
            usize,
            Showing,
            &'static [(usize, &'static str)],
        );
        const CASES: &[Case] = &[
            // The first, with the current one after it: it slides down.
            (
                3,
                2,
                0,
                &["S1", "S2"],
                1,
                Showing::Same,
                &[(0, "P1"), (1, "P2")],
            ),
            // The middle, current before it: unchanged.
            (
                3,
                0,
                1,
                &["S0", "S2"],
                0,
                Showing::Same,
                &[(0, "P0"), (1, "P2")],
            ),
            // The last, current before it.
            (
                3,
                1,
                2,
                &["S0", "S1"],
                1,
                Showing::Same,
                &[(0, "P0"), (1, "P1")],
            ),
            // The current one in the middle: the next slides into its place.
            (
                3,
                1,
                1,
                &["S0", "S2"],
                1,
                Showing::Other,
                &[(0, "P0"), (1, "P2")],
            ),
            // The current one, last: clamp to the new last.
            (
                3,
                2,
                2,
                &["S0", "S1"],
                1,
                Showing::Other,
                &[(0, "P0"), (1, "P1")],
            ),
            // The current one, first.
            (2, 0, 0, &["S1"], 0, Showing::Other, &[(0, "P1")]),
        ];
        for (count, current, index, left, after, showing, recall) in CASES {
            let mut all = spaces(*count, *current);
            let removed = remove(&mut all, *index).expect("removable");
            assert_eq!(names(&all), *left, "names, case {count}/{current}/{index}");
            assert_eq!(
                all.current, *after,
                "current, case {count}/{current}/{index}"
            );
            assert_eq!(removed.now_current, *after);
            assert_eq!(removed.showing, *showing);
            assert_eq!(removed.index, *index);
            assert_eq!(removed.space.name, format!("S{index}"));
            assert_eq!(
                places(&all),
                *recall,
                "recall, case {count}/{current}/{index}"
            );
        }
    }

    #[test]
    fn the_last_space_and_a_missing_one_are_refused_and_change_nothing() {
        const CASES: &[(usize, usize, Refused)] = &[
            (1, 0, Refused::Last),
            (3, 3, Refused::Missing),
            (1, 4, Refused::Missing),
        ];
        for (count, index, why) in CASES {
            let mut all = spaces(*count, 0);
            let before = all.clone();
            assert_eq!(remove(&mut all, *index), Err(*why));
            assert_eq!(all, before);
        }
    }
}
