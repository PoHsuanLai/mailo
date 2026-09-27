//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

mod add_account;
mod app;
mod brand;
mod checks;
mod clock;
mod command;
mod compose;
mod contacts;
mod data;
mod debounce;
mod destroy;
mod field;
mod files;
mod folder_open;
mod follow_up;
mod frame;
mod history;
mod host;
mod hover;
mod invite;
mod keyboard;
mod launch;
mod launcher_count;
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
mod picks;
mod press;
mod print;
mod reading;
mod receipt;
mod revisions;
mod row;
mod rules;
mod server_search;
mod sidebar;
mod space_editor;
mod start;
mod style;
mod switch;
mod text;
mod unsubscribe;
mod view_groups;
mod views;
mod window;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod shell_tests;

pub use launch::run;

/// The window on Blitz, for a test to drive through `ds_native::Harness`.
pub mod native {
    pub use super::brand::BrandCache;
    pub use super::clock::WallClock;
    pub use super::compose::Dictionaries;
    pub use super::follow_up::Notices;
    pub use super::launch::native::{contexts, root};
    pub use super::original::{Browse, Consent, Fetch, FetchImage, Got, Original};
    pub use super::print::Printer;
    pub use super::reading::OriginalFrame;
    pub use super::revisions::Revisions;
    pub use super::server_search::{Search, ServerSearcher};
    pub use super::window::{Ask, MessageOpen, OpenWindow, Windows, message_root};
}

pub use start::{Start, mailto_of, open_thread, start_mailto, start_of};
