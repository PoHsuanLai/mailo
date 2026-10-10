//! The key hints the command panel and the window's menu rows write beside a command, spelled
//! for the Mac. They are the one place those glyphs are hand-written, so the day quire spells a
//! chord for the platform there is one list to replace.
//!
//! TODO(quire v0.3.7): take these from `ds_core::command::chord_text` over the command's chord,
//! so a Linux window reads "Ctrl+N". That needs the window's keymap at the row; the functions
//! that build rows are tested without a runtime, so it is not looked up there yet.

/// Compose (⌘N).
pub(super) const COMPOSE: &str = "\u{2318}N";

/// Hide sidebar (⌃⌘S).
pub(super) const HIDE_SIDEBAR: &str = "\u{2303}\u{2318}S";

/// Print conversation (⌘P).
pub(super) const PRINT: &str = "\u{2318}P";

/// Open the focused conversation in its own window (⇧ Enter).
pub(super) const OPEN_IN_WINDOW: &str = "\u{21e7} Enter";
