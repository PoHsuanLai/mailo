//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

mod app;
mod command;
mod compose;
mod data;
mod field;
mod frame;
mod history;
mod hover;
mod icon;
mod launch;
mod list;
mod list_search;
mod marked;
mod menu;
mod menus;
mod motion;
mod ops;
mod page;
mod paint;
mod reading;
mod row;
mod sidebar;
mod space_editor;
mod style;
mod switch;
mod text;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod shell_tests;

pub use launch::run;
