//! `mailo` — the command line. The Dioxus shell will call the same store methods.

// The window has two frontends and draws with exactly one. Only `ui`'s launch and host seam
// read the choice; everything else here is the same code under either.
#[cfg(not(any(feature = "webview", feature = "native")))]
compile_error!(
    "mail-app needs a frontend: build with the default `webview` feature, or with \
     `--no-default-features --features native`"
);
#[cfg(all(feature = "webview", feature = "native"))]
compile_error!(
    "mail-app draws with one frontend at a time: `webview` and `native` are both on. \
     For Blitz, build with `--no-default-features --features native`"
);

pub mod account;
pub mod appearance;
pub mod attach;
pub mod cli;
pub mod compose;
pub mod contacts;
pub mod discover;
pub mod editor;
pub mod export;
pub mod folder;
pub mod import;
pub mod invite;
pub mod ipc;
pub mod notify;
pub mod password;
pub mod pgp;
pub mod print;
mod provider;
pub mod query;
pub mod reader;
pub mod receipt;
pub mod rules;
pub mod search;
pub mod smime;
pub mod snooze;
pub mod space;
pub mod sync;
pub mod template;
mod today;
pub mod trust;
pub mod ui;
pub mod undo;
pub mod unsubscribe;
pub mod view;
