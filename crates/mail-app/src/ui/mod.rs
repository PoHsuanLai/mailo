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
mod page;
mod pgp;
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

/// Stand-ins for the two Lucide glyphs mailo drew that quire's [`ds::Icon`] does not have yet,
/// `printer` (the reader's Print) and `folder-input` (Move to…, a rule's File action). A gap
/// reported to quire, not a local copy: when quire adds them, these two lines change and
/// nothing else does.
pub(crate) const PRINTER: ds::Icon = ds::Icon::File;
/// See [`PRINTER`].
pub(crate) const FOLDER_INPUT: ds::Icon = ds::Icon::Folder;
pub use start::{Start, open_thread, start_of};
