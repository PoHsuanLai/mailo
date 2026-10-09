//! The window's chords, in the Mac's terms (quire's `design/27-HIG-PARITY.md` section 6.2).
//!
//! Command is the main modifier. A session that maps Command to Control (Toshy) delivers it as
//! Ctrl, a Mac or a session that keeps it delivers Meta, so ⌘ is exactly one of the two, and
//! ⌃⌘ is both (Toshy swaps the physical Control to Meta, a Mac sends both as they are). Shift
//! and Option make a different chord, never this one: ⇧⌘Z is redo, not undo.
//!
//! | Before   | Now | Does                         |
//! | -------- | --- | ---------------------------- |
//! | Ctrl T   | ⌘K  | the search bar               |
//! | Ctrl S   | ⌃⌘S | hide or show the sidebar     |
//! | Ctrl 1-9 | ⌘1-9 | switch to Space n           |
//! | Ctrl F   | ⌘F  | find in the open message     |
//! | Ctrl P   | ⌘P  | print the open message       |
//! | Ctrl Z   | ⌘Z  | undo                         |
//! |          | ⌘,  | Settings                     |
//! | c        | ⌘N (and c) | a new message         |
//! | Ctrl Enter | ⌘Return | send                   |

use dioxus::prelude::Modifiers;

/// A chord the window answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Chord {
    /// ⌘K: the keyboard to the search bar.
    CommandMenu,
    /// ⌃⌘S: hide or show the sidebar.
    ToggleSidebar,
    /// ⌘1 to ⌘9: Space n, from 0.
    SwitchSpace(usize),
    /// ⌘F: find in the open message.
    Find,
    /// ⌘P: print the open message.
    Print,
    /// ⌘Z: undo.
    Undo,
    /// ⌘,: Settings, where every Mac app keeps its settings.
    Settings,
}

/// Whether ⌘ is held, and only ⌘: Ctrl or the command key alone, no Option. The command key
/// is Meta or Super as the window reports it; quire's `command_keys` folds them into one.
pub(in crate::ui) fn command(modifiers: Modifiers) -> bool {
    let held = ds::prelude::command_keys(modifiers);
    (held.ctrl() != held.meta()) && !held.alt()
}

/// Whether ⌃⌘ is held together, no Option.
pub(in crate::ui) fn control_command(modifiers: Modifiers) -> bool {
    let held = ds::prelude::command_keys(modifiers);
    held.ctrl() && held.meta() && !held.alt()
}

/// The chord `key` means with `modifiers` held, if it is one. `key` is the DOM's name for it.
pub(in crate::ui) fn chord(key: &str, modifiers: Modifiers) -> Option<Chord> {
    let lower = key.to_lowercase();
    if control_command(modifiers) {
        return (lower == "s" && !modifiers.shift()).then_some(Chord::ToggleSidebar);
    }
    if !command(modifiers) || modifiers.shift() {
        return None;
    }
    match lower.as_str() {
        "k" => Some(Chord::CommandMenu),
        "f" => Some(Chord::Find),
        "p" => Some(Chord::Print),
        "z" => Some(Chord::Undo),
        "," => Some(Chord::Settings),
        digit => super::switch::space_key(digit).map(Chord::SwitchSpace),
    }
}

#[cfg(test)]
mod tests {
    use super::{Chord, chord, command, control_command};
    use dioxus::prelude::Modifiers;

    const CTRL: Modifiers = Modifiers::CONTROL;
    const META: Modifiers = Modifiers::META;
    const SUPER: Modifiers = Modifiers::SUPER;

    /// ⌘ is Ctrl or the command key alone, no Option, and ⌃⌘ is both. Super is the command key
    /// as a window reports it.
    #[test]
    fn which_modifiers_are_command_and_control_command() {
        const ALT: Modifiers = Modifiers::ALT;
        const CASES: &[(&str, Modifiers, bool, bool)] = &[
            ("ctrl", CTRL, true, false),
            ("meta", META, true, false),
            ("super", SUPER, true, false),
            ("ctrl and meta", CTRL.union(META), false, true),
            ("ctrl and super", CTRL.union(SUPER), false, true),
            ("nothing", Modifiers::empty(), false, false),
            ("ctrl and option", CTRL.union(ALT), false, false),
            (
                "ctrl, meta and option",
                CTRL.union(META).union(ALT),
                false,
                false,
            ),
        ];
        for (name, held, is_command, is_control_command) in CASES {
            assert_eq!(command(*held), *is_command, "{name}: command");
            assert_eq!(
                control_command(*held),
                *is_control_command,
                "{name}: control command"
            );
        }
    }

    #[test]
    fn the_standard_map_is_what_each_chord_means_under_either_command() {
        const CASES: &[(&str, Option<Chord>, &str)] = &[
            ("k", Some(Chord::CommandMenu), "the search bar"),
            (
                "K",
                Some(Chord::CommandMenu),
                "the search bar, shifted letter",
            ),
            ("f", Some(Chord::Find), "find"),
            ("p", Some(Chord::Print), "print"),
            ("z", Some(Chord::Undo), "undo"),
            (",", Some(Chord::Settings), "settings"),
            ("1", Some(Chord::SwitchSpace(0)), "the first Space"),
            ("2", Some(Chord::SwitchSpace(1)), "the second Space"),
            ("9", Some(Chord::SwitchSpace(8)), "the ninth Space"),
            ("0", None, "there is no Space 0"),
            ("10", None, "two digits are not a key"),
            ("a", None, "a letter is not a Space"),
            ("", None, "no key"),
            ("t", None, "⌘T is the Mac's fonts, not ours"),
            ("s", None, "⌘S is Save"),
        ];
        for held in [CTRL, META, SUPER] {
            for (key, want, name) in CASES {
                assert_eq!(chord(key, held), *want, "{held:?} {key:?}: {name}");
            }
            assert_eq!(chord("z", held | Modifiers::SHIFT), None, "⇧⌘Z is redo");
        }
        assert_eq!(chord("s", CTRL | META), Some(Chord::ToggleSidebar));
        assert_eq!(chord("s", CTRL | SUPER), Some(Chord::ToggleSidebar));
        assert_eq!(chord("k", Modifiers::empty()), None);
    }
}
