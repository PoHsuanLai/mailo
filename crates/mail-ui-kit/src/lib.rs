//! What the window's parts are drawn from: the shell state they read (`view`, `space`,
//! `appearance`, `keymap`), the hooks and seams every one of them reaches the world through
//! (`live`, `host`, `motion`, `ask`), and the controls they share (`menu`, `common`, `press`).
//!
//! It is below every part of the window and names none: a part that needs another's work says
//! so through [`intent`] or is handed what it needs as a prop, and what two parts share lives
//! here. Like `mail-ui` it names no `mail-core`, `mail-store` or `mail-runtime`.

pub mod actions;
pub mod appearance;
pub mod bin;
pub mod clock;
pub mod common;
pub mod configured;
pub mod data;
pub mod debounce;
pub mod frame_selection;
pub mod history;
pub mod host;
pub mod hover;
pub mod intent;
pub mod keymap;
pub mod list_query;
pub mod list_search;
pub mod live;
pub mod menu;
pub mod motion;
pub mod ops;
pub mod original;
pub mod pasted_image;
pub mod pick;
pub mod prefs;
pub mod press;
pub mod quick;
pub mod remedy;
pub mod saved;
pub mod sensitive_copy;
pub mod space;
pub mod text;
pub mod today;
pub mod view;

/// The log target of the window's few traces of what it was asked from outside (a banner's
/// click, a second launch's hand-off). They go to stderr beside the warnings: a window has no
/// other way to say what a click did, and the desktop scenarios (`dev/scenarios`) read them there.
pub const TRACE: &str = "mailo::trace";
