//! What the window knows and says, with none of the drawing: nothing here names dioxus, quire's
//! `ds` or a renderer, so a crate that only needs a value of the window's (the command line reads
//! its settings files) does not build the toolkit to get it.
//!
//! - [`settings`]: the settings files the window and the command line read.
//! - [`damage`]: the settings files found damaged at launch.
//! - [`copy`]: every word the window shows, by kind.
//! - [`add_account`]: the account service the Add account window hosts, and the add itself.
//! - [`fetching`]: what is said of the fetch links.
//! - [`selection`]: several conversations picked at once.

pub mod add_account;
pub mod copy;
pub mod damage;
pub mod fetching;
pub mod selection;
pub mod settings;
