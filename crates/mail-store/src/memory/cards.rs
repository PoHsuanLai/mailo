//! People's cards in memory. Kept in step with `sqlite/cards.rs`, which the parity tests in
//! `tests/store/cards.rs` hold it to.

use super::Inner;
use crate::contact::{CardId, PersonCard};

impl Inner {
    pub(super) fn every_card(&self) -> Vec<PersonCard> {
        let mut out: Vec<PersonCard> = self.cards.values().cloned().collect();
        // SQLite's `ORDER BY name COLLATE NOCASE, id`: `NOCASE` folds ASCII letters only.
        out.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    pub(super) fn one_card(&self, id: &CardId) -> Option<PersonCard> {
        self.cards.get(id).cloned()
    }

    /// The first card by id listing `address`, as SQLite's `ORDER BY card LIMIT 1` finds it.
    pub(super) fn card_listing(&self, address: &str) -> Option<PersonCard> {
        let address = crate::contact::normalise(address)?;
        self.cards
            .values()
            .find(|card| card.addresses.contains(&address))
            .cloned()
    }

    pub(super) fn write_card(&mut self, card: &PersonCard) {
        self.cards.insert(card.id.clone(), card.clone());
    }

    pub(super) fn drop_card(&mut self, id: &CardId) -> bool {
        self.cards.remove(id).is_some()
    }
}
