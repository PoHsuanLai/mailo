//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

mod app;
mod composer;
mod data;
mod launch;
mod list;
mod menus;
mod ops;
mod reading;
mod row;
mod sidebar;
mod style;
mod text;

#[cfg(test)]
mod fixtures;

pub use launch::run;
