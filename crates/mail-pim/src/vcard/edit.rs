//! What an editor shows of a person's card, and an edit made onto the card as it is.
//!
//! The fields the editor shows are replaced and everything else on the card stays as it came: a
//! birthday, a property nothing here reads, an address's type. The same edit is made onto the
//! card the user opened and, when a book's server has changed the card since, onto the server's.

use super::{Card, Email, Phone, Postal};

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

/// What `card` says, for the editor.
pub fn details_of(card: &Card) -> Details {
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
pub fn apply(mut card: Card, details: &Details) -> Card {
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
