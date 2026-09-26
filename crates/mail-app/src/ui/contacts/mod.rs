//! Contacts in the window: the book the composer suggests from, a sheet to look through it, and
//! the sender card's part in it.
//!
//! [`book`] is every question and every write, as functions of a store, and [`groups`] the
//! same for contact groups; the sheet and the card only draw what they answer. CardDAV sync stays on the command line — it needs a URL and a
//! password — and the sheet says so, with the command.

pub(in crate::ui) mod book;
mod card;
mod group_rows;
pub(in crate::ui) mod groups;
mod sheet;

pub(in crate::ui) use card::ContactPart;
pub(in crate::ui) use sheet::ContactsSheet;

use crate::view::Shell;
use dioxus::prelude::*;

/// Open the Contacts sheet with an empty filter, and put the cursor in it.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().contacts = Some(String::new());
    crate::ui::host::Host::focus_next_frame(".book-find input");
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().contacts = None;
    crate::ui::host::Host::focus_app();
}

#[cfg(test)]
mod groups_tests;
#[cfg(test)]
mod sheet_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;
