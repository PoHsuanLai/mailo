//! People's cards as functions of a store: what a card says, as the address book's editor shows
//! it, and the edits a person makes — a new card, a card changed, a card forgotten, a sender
//! added from their mail — and the photo the reader shows for a sender.
//!
//! A card is kept as vCard ([`PersonCard::vcard`]), and an edit is made onto the card as it is:
//! the fields the editor shows are replaced, and everything else on the card — a birthday, a
//! property nothing here reads, an address's type — stays as it came. A synced card is marked
//! edited, and the next sync of its book writes it back over the etag it came with; a synced card
//! is its address book's to delete, as a synced group is.
//!
//! The book autocomplete ranks follows: every address a card lists is an entry named as the card
//! is, the user's for a card made here and its book's for a synced one, and an address taken off a
//! card goes back to the mail history that knew it, or away when nothing did.

use crate::error::{CoreError, Logged};
use mail_domain::filter::search_tokens;
use mail_pim::vcard::{self, Card, Email, Phone, Postal};
use mail_runtime::carddav::{addresses_of, person_card};
use mail_store::{CardId, Contact, Edit, GroupHome, Origin, PersonCard, Store, StoreError};

/// What a card says, as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Details {
    /// `FN`.
    pub name: String,
    /// Each `EMAIL`, in the card's order.
    pub emails: Vec<String>,
    /// Each `TEL`, as written.
    pub phones: Vec<String>,
    /// Each `ADR`.
    pub postal: Vec<Postal>,
    /// `ORG`'s organisation, its units left as they are.
    pub org: String,
    pub note: String,
    /// `PHOTO`: a `data:` URI, or a link the card gave.
    pub photo: Option<String>,
}

impl Details {
    /// The details of a card that says only `name` and `address`: what Add to Contacts makes.
    pub fn of(name: &str, address: &str) -> Self {
        Self {
            name: name.trim().to_owned(),
            emails: vec![address.trim().to_owned()],
            ..Self::default()
        }
    }
}

/// Why a change to a card was not made.
#[derive(Debug)]
pub enum PersonError {
    /// A card needs a name or an address.
    Empty,
    /// What was typed for an address is not one.
    BadAddress(String),
    /// There is no such card any more.
    Gone,
    /// A synced card is its address book's to delete.
    InBook {
        name: String,
    },
    Core(CoreError),
}

impl From<CoreError> for PersonError {
    fn from(error: CoreError) -> Self {
        PersonError::Core(error)
    }
}

impl From<StoreError> for PersonError {
    fn from(error: StoreError) -> Self {
        PersonError::Core(error.into())
    }
}

/// The card's text, read.
fn parsed(card: &PersonCard) -> Card {
    vcard::parse(&card.vcard)
        .into_iter()
        .next()
        // An empty card when it does not parse, which an edit then fills.
        .unwrap_or_default()
}

/// What `card` says, for the editor.
pub fn details(card: &PersonCard) -> Details {
    let card = parsed(card);
    Details {
        // Unnamed cards are listed by an address, but their name field starts empty.
        name: card.display_name().unwrap_or_default(),
        emails: card.emails.iter().map(|e| e.address.clone()).collect(),
        phones: card.phones.iter().map(|p| p.number.clone()).collect(),
        postal: card.postal(),
        // A card with no organisation shows an empty field.
        org: card.org.first().cloned().unwrap_or_default(),
        // A card with no note shows an empty field.
        note: card.note.clone().unwrap_or_default(),
        photo: card.photo.clone(),
    }
}

/// `details` made onto `card`: the fields the editor shows replaced, and an address or a number
/// kept with its type and preference where it is still there. `REV` is dropped rather than left
/// saying the card is older than it is, and `N` when the name changed, rather than left saying
/// another.
fn apply(mut card: Card, details: &Details) -> Card {
    let name = details.name.trim();
    if card.display_name().as_deref() != Some(name) {
        card.name = None;
    }
    card.formatted_name = (!name.is_empty()).then(|| name.to_owned());
    let held = std::mem::take(&mut card.emails);
    card.emails = details
        .emails
        .iter()
        .map(|address| address.trim())
        .filter(|address| !address.is_empty())
        .map(|address| {
            held.iter()
                .find(|e| e.address.eq_ignore_ascii_case(address))
                .cloned()
                .unwrap_or_else(|| Email {
                    address: address.to_owned(),
                    kinds: Vec::new(),
                    pref: None,
                })
        })
        .collect();
    let held = std::mem::take(&mut card.phones);
    card.phones = details
        .phones
        .iter()
        .map(|number| number.trim())
        .filter(|number| !number.is_empty())
        .map(|number| {
            held.iter()
                .find(|p| p.number == number)
                .cloned()
                .unwrap_or_else(|| Phone {
                    number: number.to_owned(),
                    kinds: Vec::new(),
                    pref: None,
                })
        })
        .collect();
    card.set_postal(&details.postal);
    let org = details.org.trim();
    if card.org.first().map(String::as_str) != Some(org) {
        card.org = if org.is_empty() {
            Vec::new()
        } else {
            vec![org.to_owned()]
        };
    }
    let note = details.note.trim();
    card.note = (!note.is_empty()).then(|| note.to_owned());
    card.photo = details
        .photo
        .clone()
        .filter(|photo| !photo.trim().is_empty());
    card.revision = None;
    card
}

/// Every address of `details` checked, or the first that is not one.
fn check(details: &Details) -> Result<(), PersonError> {
    for typed in &details.emails {
        if !typed.trim().is_empty() && mail_store::contact::normalise(typed).is_none() {
            return Err(PersonError::BadAddress(typed.trim().to_owned()));
        }
    }
    let named = !details.name.trim().is_empty();
    if !named && details.emails.iter().all(|e| e.trim().is_empty()) {
        return Err(PersonError::Empty);
    }
    Ok(())
}

/// The origin a card's addresses have in the book: the user's for one made here, its book's for
/// a synced one.
fn origin_of(home: &GroupHome) -> Origin {
    match home {
        GroupHome::Local => Origin::Manual,
        GroupHome::Book { url, .. } => Origin::Book(format!("carddav:{url}")),
    }
}

/// Bring the book autocomplete ranks in step with `card`, which listed `before`: its addresses
/// named as it is, and those it no longer lists let go of.
fn follow(store: &dyn Store, card: &PersonCard, before: &[String]) -> Result<(), StoreError> {
    let origin = origin_of(&card.home);
    let name = vcard::parse(&card.vcard)
        .into_iter()
        .next()
        .and_then(|parsed| parsed.display_name());
    for address in &card.addresses {
        let theirs = store.contact(address)?;
        // Typed by the user: a synced card does not take the entry over.
        if origin != Origin::Manual && theirs.is_some_and(|c| c.origin == Origin::Manual) {
            continue;
        }
        store.put_contact(address, name.as_deref(), &origin)?;
    }
    for address in before.iter().filter(|a| !card.addresses.contains(a)) {
        let_go(store, address)?;
    }
    Ok(())
}

/// Let go of `address`, which a card no longer lists: unless another card does, its entry goes
/// back to the mail history that knew it, or away when nothing did.
fn let_go(store: &dyn Store, address: &str) -> Result<(), StoreError> {
    if store.card_of(address)?.is_some() {
        return Ok(());
    }
    let Some(contact) = store.contact(address)? else {
        return Ok(());
    };
    if contact.origin == Origin::History {
        return Ok(());
    }
    if contact.written.count == 0 && contact.received.count == 0 {
        store.delete_contact(address)?;
    } else {
        store.put_contact(address, None, &Origin::History)?;
    }
    Ok(())
}

/// Every card, with a word of its name or an address beginning each word of `filter`, by name.
pub fn listed(store: &dyn Store, filter: &str) -> Result<Vec<PersonCard>, CoreError> {
    let wanted = search_tokens(filter);
    Ok(store
        .cards()?
        .into_iter()
        .filter(|card| {
            let words: Vec<String> = search_tokens(&card.name)
                .into_iter()
                .chain(card.addresses.iter().flat_map(|a| search_tokens(a)))
                .collect();
            wanted
                .iter()
                .all(|want| words.iter().any(|word| word.starts_with(want.as_str())))
        })
        .collect())
}

/// A new card made here, saying `details`.
pub fn create(store: &dyn Store, details: &Details) -> Result<PersonCard, PersonError> {
    check(details)?;
    let uid = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let card = apply(
        Card {
            uid: Some(uid.clone()),
            ..Card::new()
        },
        details,
    );
    let person = person_card(
        &card,
        vcard::write(&card),
        CardId::local(&uid),
        GroupHome::Local,
    );
    store.put_card(&person)?;
    follow(store, &person, &[])?;
    Ok(person)
}

/// The card `id`, changed to say `details`. A synced one is marked edited, so the next sync of
/// its book writes it back.
pub fn save(store: &dyn Store, id: &CardId, details: &Details) -> Result<PersonCard, PersonError> {
    check(details)?;
    let held = store.card(id)?.ok_or(PersonError::Gone)?;
    let card = apply(parsed(&held), details);
    let mut home = held.home.clone();
    if let GroupHome::Book { edit, .. } = &mut home {
        *edit = Edit::Edited;
    }
    let person = person_card(&card, vcard::write(&card), id.clone(), home);
    store.put_card(&person)?;
    follow(store, &person, &held.addresses)?;
    Ok(person)
}

/// Forget a card made here, and let go of its addresses. A synced card is its address book's to
/// delete.
pub fn forget(store: &dyn Store, id: &CardId) -> Result<(), PersonError> {
    let held = store.card(id)?.ok_or(PersonError::Gone)?;
    if held.home != GroupHome::Local {
        return Err(PersonError::InBook { name: held.name });
    }
    store.delete_card(id)?;
    for address in &held.addresses {
        let_go(store, address)?;
    }
    Ok(())
}

/// What Add to Contacts came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// A new card, for a sender nobody's card listed.
    New(PersonCard),
    /// The card that already lists the sender, left as it was.
    Already(PersonCard),
}

/// A card for the sender of a message, `name` at `address`, unless a card lists them already.
/// The name is the book's when the book has one, else the mail's.
pub fn add_sender(store: &dyn Store, address: &str, name: &str) -> Result<Added, PersonError> {
    if let Some(held) = store.card_of(address)? {
        return Ok(Added::Already(held));
    }
    let known = store
        .contact(address)?
        .and_then(|contact: Contact| contact.name);
    let name = known.as_deref().unwrap_or(name);
    create(store, &Details::of(name, address)).map(Added::New)
}

/// The photo to show for `address`: its card's, when it has one written into the card. A
/// photo the card only links to is not fetched: that would tell its host the mail was read.
pub fn photo_of(store: &dyn Store, address: &str) -> Option<String> {
    let card = store
        .card_of(address)
        .or_log("a sender's card could not be read")
        .flatten()?;
    parsed(&card)
        .photo
        .filter(|photo| photo.starts_with("data:image/"))
}

/// `card` as a person's card made here, for an import: its `UID` kept, or one given it, and a
/// card imported before with that `UID` replaced. A card that says nothing — no name, no
/// address — is not kept.
pub(crate) fn import_card(store: &dyn Store, card: &Card) -> Result<bool, StoreError> {
    if card.display_name().is_none() && addresses_of(card).is_empty() {
        return Ok(false);
    }
    let mut card = card.clone();
    let uid = card
        .uid
        .get_or_insert_with(|| format!("urn:uuid:{}", uuid::Uuid::new_v4()))
        .clone();
    let id = CardId::local(&uid);
    // A re-import replaces its own card; the addresses it no longer lists are let go of.
    let before = store
        .card(&id)?
        .map(|held| held.addresses)
        // A card gone from the store lists no addresses.
        .unwrap_or_default();
    let person = person_card(&card, vcard::write(&card), id, GroupHome::Local);
    store.put_card(&person)?;
    for address in before.iter().filter(|a| !person.addresses.contains(a)) {
        let_go(store, address)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
