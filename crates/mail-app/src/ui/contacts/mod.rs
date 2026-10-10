//! Contacts in the window: the book the composer suggests from, a page of Settings to look
//! through it, and the sender card's part in it.
//!
//! [`book`] is every question and every write, as functions of a store, and [`groups`] the
//! same for contact groups; the page and the card only draw what they answer. CardDAV sync stays
//! on the command line — it needs a URL and a password — and the page says so, with the command.

pub(in crate::ui) mod book;
mod group_edit;
mod group_rows;
pub(in crate::ui) mod groups;
mod page;

pub(in crate::ui) use page::ContactsPage;

#[cfg(test)]
mod page_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;
