//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

mod add_account;
mod app;
mod command;
mod compose;
mod contacts;
mod data;
mod debounce;
mod field;
mod files;
mod folder_open;
mod frame;
mod history;
mod host;
mod hover;
mod invite;
mod launch;
mod list;
mod list_query;
mod list_search;
mod marked;
mod menu;
mod menus;
mod motion;
mod move_to;
mod ops;
mod original;
mod page;
mod pgp;
mod pick;
mod press;
mod print;
mod reading;
mod receipt;
mod row;
mod rules;
mod sidebar;
mod space_editor;
mod start;
mod style;
mod switch;
mod text;
mod unsubscribe;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod shell_tests;

pub use launch::run;

/// The window on Blitz, for a test to drive through `ds_native::Harness`.
pub mod native {
    pub use super::launch::native::{contexts, root};
    pub use super::original::{Browse, Consent, Fetch, FetchImage, Got, Original};
    pub use super::print::Printer;
    pub use super::reading::OriginalFrame;
}

pub use start::{Start, open_thread, start_of};
