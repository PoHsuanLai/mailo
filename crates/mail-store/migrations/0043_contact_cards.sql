-- People's cards (`crate::contact::card`): a vCard of a person with every address, telephone,
-- postal address, photo and note it holds, made here, imported, or synced from a CardDAV book.
-- The `contacts` table stays the book autocomplete ranks, one row an address; a card is the
-- person behind some of those rows.
--
-- id:        `local:<uid>` for a card made or imported here, the card's URL for a synced one.
-- name:      what the card is listed by.
-- addresses: serde(Vec<String>): its addresses, normalised, in the card's order.
-- vcard:     the card itself, as vCard 4.0, written back as it is.
-- home:      serde(GroupHome): local, or the book and card it syncs with and whether it holds an
--            edit the server has not been sent.
CREATE TABLE contact_cards (
    id        TEXT PRIMARY KEY,
    name      TEXT NOT NULL,
    addresses TEXT NOT NULL,
    vcard     TEXT NOT NULL,
    home      TEXT NOT NULL
) WITHOUT ROWID;

-- Which card lists which address: how the reader finds a sender's photo, and Add to Contacts
-- whether the sender already has a card.
CREATE TABLE contact_card_addresses (
    address TEXT NOT NULL,
    card    TEXT NOT NULL REFERENCES contact_cards(id) ON DELETE CASCADE,
    PRIMARY KEY (address, card)
) WITHOUT ROWID;
