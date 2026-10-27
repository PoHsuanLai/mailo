//! People's cards: a vCard of one person, with everything it says about them — every address,
//! telephone number and postal address, a photo, a note — made here, imported, or synced from a
//! CardDAV address book.
//!
//! The book autocomplete ranks stays one entry an address ([`crate::Contact`]); a card is the
//! person behind some of those entries, and is what the address book lists and edits. The store
//! keeps the card as vCard text and does not read it: `mail_pim::vcard` does, above it. What it
//! keeps beside the text is what it is asked by — the name the card is listed by and its
//! addresses, so the reader can find a sender's card, and its photo, by address.

use super::GroupHome;
use serde::{Deserialize, Serialize};

/// What a card is known by here, as a group is ([`crate::GroupId`]): a synced card by its URL,
/// one made or imported here by `local:` and its `UID`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CardId(pub String);

impl CardId {
    /// The id of a card made here whose `UID` is `uid`.
    pub fn local(uid: &str) -> Self {
        Self(format!("local:{uid}"))
    }
}

impl std::fmt::Display for CardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One person's card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonCard {
    pub id: CardId,
    /// What the card is listed by: its formatted name, else its first address.
    pub name: String,
    /// Its addresses, normalised, in the card's order.
    pub addresses: Vec<String>,
    /// The card as its text is kept: a made or imported one as vCard 4.0, a synced one as the
    /// server sent it, whatever its version.
    pub vcard: String,
    /// Where it lives, which decides where an edit goes: a group's home serves a card the same.
    pub home: GroupHome,
}

impl PersonCard {
    /// A card listed by `name`, with `addresses`, whose text is `vcard`, living at `home`.
    pub fn new(
        id: CardId,
        name: String,
        addresses: Vec<String>,
        vcard: String,
        home: GroupHome,
    ) -> Self {
        Self {
            id,
            name,
            addresses,
            vcard,
            home,
        }
    }
}
