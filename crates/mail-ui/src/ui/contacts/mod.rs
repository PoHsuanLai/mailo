//! Contacts in the window: the book the composer suggests from, a person edited from their name
//! in mail, and the book in and out as vCard from ⌘K.
//!
//! [`book`] is every question and every write, as functions of a client, and [`groups`] the
//! same for the groups the composer offers; the editor, the commands and Settings › Contacts
//! ([`page`]) only draw and say what they answer. CardDAV sync stays on the command line: it
//! needs a URL and a password.

pub(in crate::ui) mod book;
mod cards;
mod editor;
pub(in crate::ui) mod groups;
pub(in crate::ui) mod page;

pub(in crate::ui) use cards::{export_cards, import_cards};
pub(in crate::ui) use editor::SenderName;

#[cfg(test)]
mod editor_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;
