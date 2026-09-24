//! The window's questions to the address book, as functions of a store: who to suggest for what
//! was typed, what the Contacts sheet lists, and the few writes a person can make by hand.
//!
//! Every suggestion goes through [`suggest`], so the To and Cc fields, `@` in the body and the
//! Ctrl T menu's People rank the same people in the same order. Import and export are
//! `crate::contacts`' — the functions `mailo contacts import|export` run — so the window and the
//! command line cannot disagree about what a vCard holds.

use std::path::{Path, PathBuf};

use mail_domain::filter::search_tokens;
use mail_store::{Contact, Kind, Origin, Store};

use crate::editor::Person;

/// How many people a suggestion menu offers.
pub(in crate::ui) const SUGGESTED: usize = 8;

/// What an exported book is called in the downloads directory.
pub(in crate::ui) const EXPORT_NAME: &str = "contacts.vcf";

/// The best [`SUGGESTED`] people for `typed`, best first: the store's autocomplete, which never
/// offers the user's own addresses or a no-reply sender they have not written to. Empty `typed`
/// is the top of the book. A store that cannot answer suggests nobody.
pub(in crate::ui) fn suggest(store: &dyn Store, typed: &str) -> Vec<Person> {
    store
        .contacts_matching(typed.trim(), SUGGESTED)
        .unwrap_or_default()
        .iter()
        .map(person_of)
        .collect()
}

/// A contact as a chip names it: their name, else the local part of the address.
pub(in crate::ui) fn person_of(contact: &Contact) -> Person {
    let name = match contact.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => local_part(&contact.address).to_owned(),
    };
    Person {
        name,
        address: contact.address.clone(),
    }
}

fn local_part(address: &str) -> &str {
    address.split('@').next().unwrap_or(address)
}

/// Where an entry came from, as the sheet and the sender card say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Standing {
    /// Nothing in the book: no mail has taught it and nobody added it.
    Unknown,
    /// Learned from mail; the name follows what the mail says.
    Learned,
    /// Added or named by the user. Mail never renames it.
    Added,
    /// Synced from an address book.
    Synced,
}

impl Standing {
    pub(in crate::ui) fn of(contact: Option<&Contact>) -> Self {
        match contact.map(|contact| &contact.origin) {
            None => Self::Unknown,
            Some(Origin::History) => Self::Learned,
            Some(Origin::Manual) => Self::Added,
            Some(Origin::Book(_)) => Self::Synced,
        }
    }

    /// The origin, in a few words.
    pub(in crate::ui) fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "not in contacts",
            Self::Learned => "from mail",
            Self::Added => "added by you",
            Self::Synced => "from an address book",
        }
    }

    /// What the name action is called: an entry only mail has named is added to the user's
    /// contacts; one they added or synced already is one, so its name is edited.
    pub(in crate::ui) fn name_action(&self) -> &'static str {
        match self {
            Self::Added | Self::Synced => "Edit name",
            Self::Unknown | Self::Learned => "Add to contacts",
        }
    }
}

/// One line of the Contacts sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Row {
    pub address: String,
    pub name: Option<String>,
    pub standing: Standing,
    /// Why autocomplete leaves it out, when it does.
    pub quiet: Option<&'static str>,
}

/// The whole book, hidden entries included, in the store's order, narrowed to entries with a
/// word beginning with each word of `filter` — the rule autocomplete matches by.
pub(in crate::ui) fn rows(store: &dyn Store, filter: &str) -> Result<Vec<Row>, String> {
    let book = store.contacts().map_err(|e| e.to_string())?;
    Ok(book
        .iter()
        .filter(|contact| fits(contact, filter))
        .map(|contact| Row {
            address: contact.address.clone(),
            name: contact.name.clone(),
            standing: Standing::of(Some(contact)),
            quiet: quiet(contact),
        })
        .collect())
}

fn quiet(contact: &Contact) -> Option<&'static str> {
    if contact.offered() {
        return None;
    }
    Some(match contact.kind {
        Kind::Own => "your address",
        Kind::Bulk | Kind::Person => "not suggested until you write",
    })
}

/// Whether `contact` has a word beginning with every word of `filter`, or an address beginning
/// with all of it. Words are folded as search folds them, so `mül` finds "Müller".
fn fits(contact: &Contact, filter: &str) -> bool {
    let raw = filter.trim().to_lowercase();
    if raw.is_empty() {
        return true;
    }
    if contact.address.starts_with(&raw) {
        return true;
    }
    let mut words = search_tokens(&contact.address);
    words.extend(contact.name.iter().flat_map(|name| search_tokens(name)));
    let wanted = search_tokens(filter);
    !wanted.is_empty()
        && wanted
            .iter()
            .all(|want| words.iter().any(|word| word.starts_with(want.as_str())))
}

/// Name `address` by hand, adding it to the book if it was not there. The entry becomes the
/// user's: mail never renames it again. An empty name keeps whatever name it has.
pub(in crate::ui) fn name(store: &dyn Store, address: &str, name: &str) -> Result<Contact, String> {
    let name = name.trim();
    store
        .put_contact(address, (!name.is_empty()).then_some(name), &Origin::Manual)
        .map_err(|e| e.to_string())
}

/// Forget `address`. `false` when there was nothing to forget. Mail may teach it again.
pub(in crate::ui) fn forget(store: &dyn Store, address: &str) -> Result<bool, String> {
    store.delete_contact(address).map_err(|e| e.to_string())
}

/// Read a vCard file's bytes into the book, as `mailo contacts import` does. The answer is a
/// sentence for the sheet.
pub(in crate::ui) fn import(store: &dyn Store, bytes: &[u8]) -> Result<String, String> {
    crate::contacts::import(store, bytes).map(|said| said.trim().replace('\n', ". "))
}

/// Write the book as vCard 4.0 into `dir`, beside anything already there, as
/// `mailo contacts export` writes it. Returns the file written.
pub(in crate::ui) fn export(store: &dyn Store, dir: &Path) -> Result<PathBuf, String> {
    let text = crate::contacts::export(store)?;
    crate::attach::write_new(dir, EXPORT_NAME, text.as_bytes())
}

/// The command that syncs a CardDAV address book. The window does not: it needs a URL and a
/// password, and the command line is where those are typed.
pub(in crate::ui) const SYNC_COMMAND: &str =
    "mailo contacts sync <url> [--account <address>] [--user <login>]";
