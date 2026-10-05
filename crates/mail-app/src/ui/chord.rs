//! The window's chords, in the Mac's terms (quire's `design/27-HIG-PARITY.md` section 6.2).
//!
//! Command is the main modifier. A session that maps Command to Control (Toshy) delivers it as
//! Ctrl, a Mac or a session that keeps it delivers Meta, so ⌘ is exactly one of the two, and
//! ⌃⌘ is both (Toshy swaps the physical Control to Meta, a Mac sends both as they are). Shift
//! and Option make a different chord, never this one: ⇧⌘Z is redo, not undo.
//!
//! | Before   | Now | Does                         |
//! | -------- | --- | ---------------------------- |
//! | Ctrl T   | ⌘K  | the command menu             |
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
    /// ⌘K: the command menu.
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

/// Whether ⌘ is held, and only ⌘: Ctrl or Meta alone, no Option.
pub(in crate::ui) fn command(modifiers: Modifiers) -> bool {
    (modifiers.ctrl() != modifiers.meta()) && !modifiers.alt()
}

/// Whether ⌃⌘ is held together, no Option.
pub(in crate::ui) fn control_command(modifiers: Modifiers) -> bool {
    modifiers.ctrl() && modifiers.meta() && !modifiers.alt()
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

    #[test]
    fn command_is_ctrl_or_meta_alone_and_control_command_is_both() {
        assert!(command(CTRL) && command(META));
        assert!(!command(CTRL | META) && control_command(CTRL | META));
        assert!(!command(Modifiers::empty()) && !control_command(CTRL));
        assert!(!command(CTRL | Modifiers::ALT) && !control_command(CTRL | META | Modifiers::ALT));
    }

    #[test]
    fn the_standard_map_is_what_each_chord_means_under_either_command() {
        for held in [CTRL, META] {
            for (key, want) in [
                ("k", Chord::CommandMenu),
                ("K", Chord::CommandMenu),
                ("f", Chord::Find),
                ("p", Chord::Print),
                ("z", Chord::Undo),
                (",", Chord::Settings),
                ("1", Chord::SwitchSpace(0)),
                ("9", Chord::SwitchSpace(8)),
            ] {
                assert_eq!(chord(key, held), Some(want), "{key}");
            }
            assert_eq!(chord("0", held), None);
            assert_eq!(chord("t", held), None, "⌘T is the Mac's fonts, not ours");
            assert_eq!(chord("s", held), None, "⌘S is Save");
            assert_eq!(chord("z", held | Modifiers::SHIFT), None, "⇧⌘Z is redo");
        }
        assert_eq!(chord("s", CTRL | META), Some(Chord::ToggleSidebar));
        assert_eq!(chord("k", Modifiers::empty()), None);
    }
}
