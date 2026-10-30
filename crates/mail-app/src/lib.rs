//! `mailo`, the application: a window and a command line over one store.
//!
//! The two front-ends sit side by side. `mail_ui::ui` is the window, drawn with quire over
//! the one client; `mail-cli` is the terminal. Neither names the other: what they share is
//! `mail_core`, which has no window and no terminal in it (`scripts/check-boundary.sh` holds both
//! lines). `mail-intents` is a third face, with neither: what the desktop's intent router asks of
//! mailo over D-Bus. This crate is the process over the three: the `mailo` binary (`main.rs`) is
//! the router between them, [`intents`] is where it answers `mailo intents`, and [`window`] opens
//! the window with what only the program makes: the client over the store, reaching servers as
//! [`edge`] says.

pub mod background;
pub mod edge;
pub mod intents;
pub mod window;
