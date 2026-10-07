//! The search bar's keys, as decisions: what Up, Down, Return, Tab and Escape do in the field,
//! given what the panel lists and what the field holds. Pure; the bar does what it says.

/// A key the field takes for itself rather than typing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum BarKey {
    Up,
    Down,
    Return,
    Tab,
    Escape,
}

/// The bar's key for a DOM key name, if it is one. A modified key is the window's (⌘K, ⇧Tab).
pub(in crate::ui) fn bar_key(name: &str, plain: Plain) -> Option<BarKey> {
    if plain == Plain::No {
        return None;
    }
    match name {
        "ArrowUp" => Some(BarKey::Up),
        "ArrowDown" => Some(BarKey::Down),
        "Enter" => Some(BarKey::Return),
        "Tab" => Some(BarKey::Tab),
        "Escape" => Some(BarKey::Escape),
        _ => None,
    }
}

/// Whether a key came with no modifier held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Plain {
    Yes,
    No,
}

/// Whether the field holds any text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Typed {
    Empty,
    Some,
}

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Step {
    /// Highlight the row at this index.
    Move(usize),
    /// Run the row at this index.
    Run(usize),
    /// Put the row at this index's text in the field.
    Complete(usize),
    /// Empty the field; the keyboard stays.
    Clear,
    /// Close the panel and give the keyboard back to the list.
    Leave,
    /// Nothing of the bar's: the key does what it would.
    Pass,
}

/// What `key` does with `count` rows in the panel, `active` highlighted (held to the rows), and
/// the field `typed`. Up and Down move across the sections and stop at the ends, as the ⌘K
/// menu's did; Return runs the highlighted row and Tab completes it; Escape empties the field
/// first, then leaves it.
pub(in crate::ui) fn step(key: BarKey, active: usize, count: usize, typed: Typed) -> Step {
    let last = count.checked_sub(1);
    let at = last.map(|last| active.min(last));
    match (key, at, last) {
        (BarKey::Up, Some(at), _) => Step::Move(at.saturating_sub(1)),
        (BarKey::Down, Some(at), Some(last)) => Step::Move((at + 1).min(last)),
        (BarKey::Return, Some(at), _) => Step::Run(at),
        (BarKey::Tab, Some(at), _) => Step::Complete(at),
        (BarKey::Escape, _, _) if typed == Typed::Some => Step::Clear,
        (BarKey::Escape, _, _) => Step::Leave,
        (BarKey::Up | BarKey::Down | BarKey::Return | BarKey::Tab, _, _) => Step::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::{BarKey, Plain, Step, Typed, bar_key, step};

    #[test]
    fn each_key_does_what_the_panel_and_the_field_allow() {
        use BarKey::*;
        const CASES: &[(&str, BarKey, usize, usize, Typed, Step)] = &[
            ("down moves on", Down, 0, 4, Typed::Some, Step::Move(1)),
            (
                "down stops at the last",
                Down,
                3,
                4,
                Typed::Some,
                Step::Move(3),
            ),
            ("up moves back", Up, 2, 4, Typed::Some, Step::Move(1)),
            (
                "up stops at the first",
                Up,
                0,
                4,
                Typed::Some,
                Step::Move(0),
            ),
            (
                "a stale highlight is held to the rows",
                Down,
                9,
                4,
                Typed::Some,
                Step::Move(3),
            ),
            (
                "return runs the highlight",
                Return,
                2,
                4,
                Typed::Some,
                Step::Run(2),
            ),
            (
                "return on a stale highlight runs the last",
                Return,
                7,
                3,
                Typed::Some,
                Step::Run(2),
            ),
            (
                "tab completes the highlight",
                Tab,
                0,
                4,
                Typed::Some,
                Step::Complete(0),
            ),
            (
                "no rows: return is the field's",
                Return,
                0,
                0,
                Typed::Some,
                Step::Pass,
            ),
            (
                "no rows: tab moves the focus",
                Tab,
                0,
                0,
                Typed::Some,
                Step::Pass,
            ),
            (
                "no rows: the arrows do nothing of ours",
                Down,
                0,
                0,
                Typed::Some,
                Step::Pass,
            ),
            (
                "escape empties a field with text",
                Escape,
                1,
                4,
                Typed::Some,
                Step::Clear,
            ),
            (
                "escape leaves an empty field",
                Escape,
                1,
                4,
                Typed::Empty,
                Step::Leave,
            ),
            (
                "escape leaves with no rows",
                Escape,
                0,
                0,
                Typed::Empty,
                Step::Leave,
            ),
        ];
        for (name, key, active, count, typed, want) in CASES {
            assert_eq!(step(*key, *active, *count, *typed), *want, "{name}");
        }
    }

    #[test]
    fn a_modified_key_is_the_windows() {
        assert_eq!(bar_key("ArrowDown", Plain::Yes), Some(BarKey::Down));
        assert_eq!(bar_key("Enter", Plain::Yes), Some(BarKey::Return));
        assert_eq!(bar_key("Tab", Plain::No), None);
        assert_eq!(bar_key("k", Plain::Yes), None);
    }
}
