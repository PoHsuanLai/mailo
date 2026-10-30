//! The window's questions to the address book, as functions of a store: who to suggest for what
//! was typed, the cards and groups Settings › Contacts lists and edits, and the writes a person
//! makes by hand.
//!
//! Every suggestion goes through [`suggest`], so the To and Cc fields, `@` in the body and the
//! ⌘K menu's People rank the same people in the same order. Import and export are
//! `mail_client::contacts`' — the functions `mailo contacts import|export` run — so the window and the
//! command line cannot disagree about what a vCard holds.

use mail_client::Client;
use std::path::{Path, PathBuf};

use mail_client::contacts::{Contact, Origin};

use mail_client::contacts::groups::{GroupError, Labelled, person_of};
use mail_client::contacts::people::{self, Added, Details, PersonError};
use mail_client::contacts::{CardId, Group, GroupId, PersonCard};
use mail_editor::Person;

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
    mail_client::attach::write_new(dir, EXPORT_NAME, text.as_bytes()).map_err(String::from)
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
/// A client that cannot answer lists nobody.
pub(in crate::ui) fn cards(client: &Client, filter: &str) -> Vec<PersonCard> {
    client.people().listed(filter).unwrap_or_else(|why| {
        log::warn!("the cards could not be read: {why}");
        Vec::new()
    })
}

/// One card, `None` when it is gone.
pub(in crate::ui) fn card(client: &Client, id: &CardId) -> Option<PersonCard> {
    client.stored().card(id).unwrap_or_else(|why| {
        log::warn!("a card could not be read: {why}");
        None
    })
}

/// What `id`'s card says, for its page; an empty card's details when it is gone.
pub(in crate::ui) fn details(client: &Client, id: &CardId) -> Details {
    card(client, id)
        .map(|card| people::details(&card))
        .unwrap_or_default()
}

/// Keep `details` as `id`'s card, or as a new card when there is no `id`.
pub(in crate::ui) fn keep_card(
    client: &Client,
    id: Option<&CardId>,
    details: &Details,
) -> Result<PersonCard, String> {
    match id {
        Some(id) => client.people().save(id, details),
        None => client.people().create(details),
    }
    .map_err(person_error)
}

/// Delete a card made here.
pub(in crate::ui) fn forget_card(client: &Client, id: &CardId) -> Result<(), String> {
    client.people().forget(id).map_err(person_error)
}

/// Add to Contacts: the sender's card, new or already there, and what to say about it.
pub(in crate::ui) fn add_sender(
    client: &Client,
    address: &str,
    in_mail: &str,
) -> Result<(CardId, Option<String>), String> {
    match client
        .people()
        .add_sender(address, in_mail)
        .map_err(person_error)?
    {
        Added::New(card) => Ok((
            card.id,
            Some(crate::ui::copy::toasts::added_to_contacts(&card.name)),
        )),
        Added::Already(card) => Ok((card.id, None)),
    }
}

/// The card listing `address`, when one does.
pub(in crate::ui) fn card_of(client: &Client, address: &str) -> Option<CardId> {
    client
        .stored()
        .card_of(address)
        .unwrap_or_else(|why| {
            log::warn!("a sender's card could not be read: {why}");
            None
        })
        .map(|card| card.id)
}

/// The photo a card holds for `address`, as a `data:` URL.
pub(in crate::ui) fn photo(client: &Client, address: &str) -> Option<String> {
    client.people().photo_of(address)
}

/// Every group, by name. A client that cannot answer lists none.
pub(in crate::ui) fn every_group(client: &Client) -> Vec<Group> {
    client.contacts_groups().listed("").unwrap_or_else(|why| {
        log::warn!("the groups could not be read: {why}");
        Vec::new()
    })
}

/// One group, `None` when it is gone.
pub(in crate::ui) fn group(client: &Client, id: &GroupId) -> Option<Group> {
    client.stored().group(id).unwrap_or_else(|why| {
        log::warn!("a group could not be read: {why}");
        None
    })
}

/// A new group of nobody, called `name`.
pub(in crate::ui) fn new_group(client: &Client, name: &str) -> Result<Group, String> {
    client.contacts_groups().create(name).map_err(group_error)
}

/// Call the group `id` `name`.
pub(in crate::ui) fn rename_group(client: &Client, id: &GroupId, name: &str) -> Result<(), String> {
    client
        .contacts_groups()
        .rename(id, name)
        .map(drop)
        .map_err(group_error)
}

/// Add `typed`, one address or several, to the group `id`.
pub(in crate::ui) fn add_members(client: &Client, id: &GroupId, typed: &str) -> Result<(), String> {
    client
        .contacts_groups()
        .add(id, typed)
        .map(drop)
        .map_err(group_error)
}

/// Take the member written `uri` out of the group `id`.
pub(in crate::ui) fn remove_member(client: &Client, id: &GroupId, uri: &str) -> Result<(), String> {
    client
        .contacts_groups()
        .remove(id, uri)
        .map(drop)
        .map_err(group_error)
}

/// Delete a group made here.
pub(in crate::ui) fn forget_group(client: &Client, id: &GroupId) -> Result<(), String> {
    client.contacts_groups().forget(id).map_err(group_error)
}

/// Each member of `group` as written, with what to call it: the person it names, the addresses,
/// or the member itself when it names nobody known.
pub(in crate::ui) fn members(client: &Client, group: &Group) -> Vec<(String, String)> {
    client
        .contacts_groups()
        .members(group)
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
