//! The window's questions to the address book, as functions of a client: who to suggest for what
//! was typed, and the few writes a person makes by hand.
//!
//! Every suggestion goes through [`suggest`], so the To and Cc fields, `@` in the body and the
//! ⌘K menu's People rank the same people in the same order. Import and export are
//! `mail_core::contacts`' — the functions `mailo contacts import|export` run — so the window and the
//! command line cannot disagree about what a vCard holds.

use mail_client::Client;
use std::path::{Path, PathBuf};

use mail_core::contacts::{Contact, Origin};

use mail_core::contacts::groups::person_of;
use mail_core::editor::Person;

/// How many people a suggestion menu offers.
pub(in crate::ui) const SUGGESTED: usize = 8;

/// What an exported book is called in the downloads directory.
pub(in crate::ui) const EXPORT_NAME: &str = "contacts.vcf";

/// The best [`SUGGESTED`] people for `typed`, best first: the client's autocomplete, which never
/// offers the user's own addresses or a no-reply sender they have not written to. Empty `typed`
/// is the top of the book. A client that cannot answer suggests nobody.
pub(in crate::ui) fn suggest(client: &Client, typed: &str) -> Vec<Person> {
    client
        .stored()
        .contacts_matching(typed.trim(), SUGGESTED)
        .unwrap_or_default()
        .iter()
        .map(person_of)
        .collect()
}

/// Name `address` by hand, adding it to the book if it was not there. The entry becomes the
/// user's: mail never renames it again. An empty name keeps whatever name it has.
pub(in crate::ui) fn name(client: &Client, address: &str, name: &str) -> Result<Contact, String> {
    let name = name.trim();
    client
        .stored()
        .put_contact(address, (!name.is_empty()).then_some(name), &Origin::Manual)
        .map_err(|e| e.to_string())
}

/// Read a vCard file's bytes into the book, as `mailo contacts import` does. The answer is a
/// sentence for the toast.
pub(in crate::ui) fn import(client: &Client, bytes: &[u8]) -> Result<String, String> {
    client
        .book()
        .import(bytes)
        .map(|done| {
            crate::said::contacts::imported(&done)
                .trim()
                .replace('\n', ". ")
        })
        .map_err(String::from)
}

/// Write the book as vCard 4.0 into `dir`, beside anything already there, as
/// `mailo contacts export` writes it. Returns the file written.
pub(in crate::ui) fn export(client: &Client, dir: &Path) -> Result<PathBuf, String> {
    let text = client.book().export()?;
    mail_core::attach::write_new(dir, EXPORT_NAME, text.as_bytes()).map_err(String::from)
}
