//! Every word the window shows, by kind: one module per kind, each with its case rule.
//!
//! A caller names the word by its kind, never through a re-export: `copy::menus::MARK_AS_READ`,
//! `copy::aria::SYNC_NOW`, `copy::status::downloading(3, 40)`. So the kind is read where the word
//! is used, and a word used as two kinds (a button and its tooltip) is two constants that may
//! differ.
//!
//! - A fixed word is a `pub const NAME: &str`.
//! - A word with values in it is a small `pub fn` that takes the values and returns a `String`.
//! - Ellipsis is written `…` and the apostrophe `’`, as characters, so a search finds them.
//!
//! The case rules (each module repeats its own): Title Case for buttons, menu items, tooltips,
//! titles and the accessible names of controls ("Mark as Read", "New Space", "Sync Now");
//! sentence case for sentences: toasts, help, errors, status lines and the body of a
//! confirmation. A header is one or two words ("Drafts", not "Drafts put aside").
//!
//! What is not here is held to a ratchet: `./scripts/check-copy.sh` counts the visible literals
//! still written elsewhere in the window (scripts/copy-literals.txt), and fails when one is added.
//! Tests name a word by its constant here, so rewording it does not break them.

pub mod aria;
pub mod confirm;
pub mod controls;
pub mod empty;
pub mod errors;
pub mod fields;
pub mod headers;
pub mod menus;
pub mod status;
pub mod titles;
pub mod toasts;
pub mod tooltips;
