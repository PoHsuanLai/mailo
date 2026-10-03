//! `mailo`, the application: a window and a command line over one store.
//!
//! The two front-ends sit side by side. [`ui`] is the window, drawn with quire; [`cli`] is the
//! terminal. Neither names the other: what they share is `mail_core`, which has no window and no
//! terminal in it (`scripts/check-boundary.sh` holds both lines). The `mailo` binary
//! (`main.rs`) is the router between them.

pub mod cli;
pub mod session;
pub mod ui;
