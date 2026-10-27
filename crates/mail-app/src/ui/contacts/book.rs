//! The window's questions to the address book, as functions of a store: who to suggest for what
//! was typed, the cards and groups Settings › Contacts lists and edits, and the writes a person
//! makes by hand.
//!
//! Every suggestion goes through [`suggest`], so the To and Cc fields, `@` in the body and the
//! ⌘K menu's People rank the same people in the same order. Import and export are
//! `mail_core::contacts`' — the functions `mailo contacts import|export` run — so the window and the
//! command line cannot disagree about what a vCard holds.

use std::path::{Path, PathBuf};

use mail_core::Store;
use mail_core::contacts::{Contact, Origin};

use mail_core::contacts::groups::{self, GroupError, Labelled, person_of};
use mail_core::contacts::people::{self, Added, Details, PersonError};
use mail_core::contacts::{CardId, Group, GroupId, PersonCard};
use mail_core::rich_text::Person;

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

/// Name `address` by hand, adding it to the book if it was not there. The entry becomes the
/// user's: mail never renames it again. An empty name keeps whatever name it has.
pub(in crate::ui) fn name(store: &dyn Store, address: &str, name: &str) -> Result<Contact, String> {
    let name = name.trim();
    store
        .put_contact(address, (!name.is_empty()).then_some(name), &Origin::Manual)
        .map_err(|e| e.to_string())
}

/// Read a vCard file's bytes into the book, as `mailo contacts import` does. The answer is a
/// sentence for the toast.
pub(in crate::ui) fn import(store: &dyn Store, bytes: &[u8]) -> Result<String, String> {
    mail_core::contacts::import(store, bytes)
        .map(|done| {
            crate::said::contacts::imported(&done)
                .trim()
                .replace('\n', ". ")
        })
        .map_err(String::from)
}

/// Write the book as vCard 4.0 into `dir`, beside anything already there, as
/// `mailo contacts export` writes it. Returns the file written.
pub(in crate::ui) fn export(store: &dyn Store, dir: &Path) -> Result<PathBuf, String> {
    let text = mail_core::contacts::export(store)?;
    mail_core::attach::write_new(dir, EXPORT_NAME, text.as_bytes()).map_err(String::from)
}

/// Why a card's edit was refused, as a sentence for the page.
fn person_error(error: PersonError) -> String {
    match error {
        PersonError::Empty => crate::ui::copy::errors::CONTACT_NEEDS_SOMETHING.to_owned(),
        PersonError::BadAddress(typed) => crate::ui::copy::errors::not_an_address(&typed),
        PersonError::Gone => crate::ui::copy::errors::CONTACT_GONE.to_owned(),
        PersonError::InBook { name } => crate::ui::copy::errors::in_address_book(&name),
        PersonError::Core(why) => why.to_string(),
    }
}

/// Why a group's edit was refused, as a sentence for the page.
fn group_error(error: GroupError) -> String {
    match error {
        GroupError::NoName => crate::ui::copy::errors::GROUP_NEEDS_A_NAME.to_owned(),
        GroupError::Gone => crate::ui::copy::errors::GROUP_GONE.to_owned(),
        GroupError::InBook { name } => crate::ui::copy::errors::in_address_book(&name),
        GroupError::Core(why) => why.to_string(),
    }
}

/// Every card with a word of its name or an address beginning each word of `filter`, by name.
/// A store that cannot answer lists nobody.
pub(in crate::ui) fn cards(store: &dyn Store, filter: &str) -> Vec<PersonCard> {
    people::listed(store, filter).unwrap_or_else(|why| {
        log::warn!("the cards could not be read: {why}");
        Vec::new()
    })
}

/// One card, `None` when it is gone.
pub(in crate::ui) fn card(store: &dyn Store, id: &CardId) -> Option<PersonCard> {
    store.card(id).unwrap_or_else(|why| {
        log::warn!("a card could not be read: {why}");
        None
    })
}

/// What `id`'s card says, for its page; an empty card's details when it is gone.
pub(in crate::ui) fn details(store: &dyn Store, id: &CardId) -> Details {
    card(store, id)
        .map(|card| people::details(&card))
        .unwrap_or_default()
}

/// Keep `details` as `id`'s card, or as a new card when there is no `id`.
pub(in crate::ui) fn keep_card(
    store: &dyn Store,
    id: Option<&CardId>,
    details: &Details,
) -> Result<PersonCard, String> {
    match id {
        Some(id) => people::save(store, id, details),
        None => people::create(store, details),
    }
    .map_err(person_error)
}

/// Delete a card made here.
pub(in crate::ui) fn forget_card(store: &dyn Store, id: &CardId) -> Result<(), String> {
    people::forget(store, id).map_err(person_error)
}

/// Add to Contacts: the sender's card, new or already there, and what to say about it.
pub(in crate::ui) fn add_sender(
    store: &dyn Store,
    address: &str,
    in_mail: &str,
) -> Result<(CardId, Option<String>), String> {
    match people::add_sender(store, address, in_mail).map_err(person_error)? {
        Added::New(card) => Ok((
            card.id,
            Some(crate::ui::copy::toasts::added_to_contacts(&card.name)),
        )),
        Added::Already(card) => Ok((card.id, None)),
    }
}

/// The card listing `address`, when one does.
pub(in crate::ui) fn card_of(store: &dyn Store, address: &str) -> Option<CardId> {
    store
        .card_of(address)
        .unwrap_or_else(|why| {
            log::warn!("a sender's card could not be read: {why}");
            None
        })
        .map(|card| card.id)
}

/// The photo a card holds for `address`, as a `data:` URL.
pub(in crate::ui) fn photo(store: &dyn Store, address: &str) -> Option<String> {
    people::photo_of(store, address)
}

/// Every group, by name. A store that cannot answer lists none.
pub(in crate::ui) fn every_group(store: &dyn Store) -> Vec<Group> {
    groups::listed(store, "").unwrap_or_else(|why| {
        log::warn!("the groups could not be read: {why}");
        Vec::new()
    })
}

/// One group, `None` when it is gone.
pub(in crate::ui) fn group(store: &dyn Store, id: &GroupId) -> Option<Group> {
    store.group(id).unwrap_or_else(|why| {
        log::warn!("a group could not be read: {why}");
        None
    })
}

/// A new group of nobody, called `name`.
pub(in crate::ui) fn new_group(store: &dyn Store, name: &str) -> Result<Group, String> {
    groups::create(store, name).map_err(group_error)
}

/// Call the group `id` `name`.
pub(in crate::ui) fn rename_group(
    store: &dyn Store,
    id: &GroupId,
    name: &str,
) -> Result<(), String> {
    groups::rename(store, id, name)
        .map(drop)
        .map_err(group_error)
}

/// Add `typed`, one address or several, to the group `id`.
pub(in crate::ui) fn add_members(
    store: &dyn Store,
    id: &GroupId,
    typed: &str,
) -> Result<(), String> {
    groups::add(store, id, typed).map(drop).map_err(group_error)
}

/// Take the member written `uri` out of the group `id`.
pub(in crate::ui) fn remove_member(
    store: &dyn Store,
    id: &GroupId,
    uri: &str,
) -> Result<(), String> {
    groups::remove(store, id, uri)
        .map(drop)
        .map_err(group_error)
}

/// Delete a group made here.
pub(in crate::ui) fn forget_group(store: &dyn Store, id: &GroupId) -> Result<(), String> {
    groups::forget(store, id).map_err(group_error)
}

/// Each member of `group` as written, with what to call it: the person it names, the addresses,
/// or the member itself when it names nobody known.
pub(in crate::ui) fn members(store: &dyn Store, group: &Group) -> Vec<(String, String)> {
    groups::members(store, group)
        .into_iter()
        .map(|(uri, labelled)| {
            let said = match labelled {
                Labelled::Named(person) => format!("{} <{}>", person.name, person.address),
                Labelled::Addresses(addresses) => addresses.join(", "),
                Labelled::NotFound => uri.clone(),
            };
            (uri, said)
        })
        .collect()
}

/// The largest picture a card takes: a photo rides in every sync and export of the card.
const PHOTO_LIMIT: usize = 1024 * 1024;

/// A picture file's bytes as the `data:` URL a card's `PHOTO` holds, its type from its name.
pub(in crate::ui) fn photo_url(name: &str, bytes: &[u8]) -> Result<String, String> {
    use base64::Engine as _;
    let lower = name.to_ascii_lowercase();
    let kind = match lower.rsplit_once('.').map(|(_, ext)| ext) {
        Some("png") => "png",
        Some("jpg" | "jpeg") => "jpeg",
        Some("gif") => "gif",
        Some("webp") => "webp",
        _ => return Err(crate::ui::copy::errors::NOT_A_PICTURE.to_owned()),
    };
    if bytes.len() > PHOTO_LIMIT {
        return Err(crate::ui::copy::errors::PHOTO_TOO_LARGE.to_owned());
    }
    Ok(format!(
        "data:image/{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
