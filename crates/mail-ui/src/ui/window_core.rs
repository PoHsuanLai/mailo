//! What every window that reads a conversation sets up the same way: the first window and a
//! conversation's own (`ui/window`).
//!
//! Two hooks, because the Spaces come between them: the keymap has to exist before `use_spaces`
//! takes a default one, and the reply's desk needs the Spaces that call makes.

use super::compose::{self, Desk};
use super::space::{Handle, Spaces};
use super::today::Today;
use super::view::Shell;
use super::{actions, appearance::WindowDirs, frame, hover, motion};
use dioxus::prelude::*;
use ds::prelude::Keys;

/// The handles the first hook makes, which the window's own keys and writes go through.
#[derive(Clone, Copy)]
pub(super) struct WindowCore {
    /// The window's keymap, with mailo's actions in it.
    pub keys: Keys,
}

/// The window's keymap, and the hover previews and motion every window provides once. Call it
/// before `use_spaces`.
pub(super) fn use_window_core() -> WindowCore {
    let keys = actions::use_registered();
    hover::use_hover();
    motion::use_motion();
    super::reading::selection::use_selections();
    WindowCore { keys }
}

/// A reply's desk, the spelling dictionaries a test brings, and the follow on the configuration
/// files the other windows write. `handle` is the first window's Spaces menu; another window
/// passes none and only wears the Spaces.
pub(super) fn use_window_desk(
    core: WindowCore,
    shell: Signal<Shell>,
    spaces: Signal<Spaces>,
    handle: Option<Handle>,
    today: Signal<Today>,
    dirs: Option<WindowDirs>,
) -> Desk {
    let desk = compose::use_desk(today, spaces, dirs);
    compose::use_test_dictionaries();
    frame::use_followed_configuration(shell, spaces, handle, core.keys);
    desk
}
