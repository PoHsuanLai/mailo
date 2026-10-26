//! People's cards in SQLite. Kept in step with `memory/cards.rs`, which the parity tests in
//! `tests/store/cards.rs` hold it to.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::StoreError;
use crate::contact::{CardId, PersonCard};
use rusqlite::{OptionalExtension, params};

const COLUMNS: &str = "id, name, addresses, vcard, home";

type Columns = (String, String, String, String, String);

fn columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<Columns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn card((id, name, addresses, vcard, home): Columns) -> Result<PersonCard, StoreError> {
    Ok(PersonCard {
        id: CardId(id),
        name,
        addresses: json("PersonCard.addresses", &addresses)?,
        vcard,
        home: json("PersonCard.home", &home)?,
    })
}

impl SqliteStore {
    pub(super) fn every_card(&self) -> Result<Vec<PersonCard>, StoreError> {
        // `NOCASE` folds ASCII only, which is what the in-memory store's ordering does too.
        let sql = format!("SELECT {COLUMNS} FROM contact_cards ORDER BY name COLLATE NOCASE, id");
        let db = self.reader();
        let mut stmt = db.prepare_cached(&sql)?;
        let rows = stmt.query_map([], columns)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(card(row?)?);
        }
        Ok(out)
    }

    pub(super) fn one_card(&self, id: &CardId) -> Result<Option<PersonCard>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM contact_cards WHERE id = ?1");
        let row = self
            .reader()
            .query_row(&sql, params![id.0], columns)
            .optional()?;
        row.map(card).transpose()
    }

    pub(super) fn card_listing(&self, address: &str) -> Result<Option<PersonCard>, StoreError> {
        let Some(address) = crate::contact::normalise(address) else {
            return Ok(None);
        };
        let id: Option<String> = self
            .reader()
            .query_row(
                "SELECT card FROM contact_card_addresses WHERE address = ?1 ORDER BY card LIMIT 1",
                params![address],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.one_card(&CardId(id)),
            None => Ok(None),
        }
    }

    pub(super) fn write_card(&self, card: &PersonCard) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM contact_card_addresses WHERE card = ?1",
            params![card.id.0],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO contact_cards (id, name, addresses, vcard, home)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                card.id.0,
                card.name,
                to_json("PersonCard.addresses", &card.addresses)?,
                card.vcard,
                to_json("PersonCard.home", &card.home)?,
            ],
        )?;
        for address in &card.addresses {
            tx.execute(
                "INSERT OR IGNORE INTO contact_card_addresses (address, card) VALUES (?1, ?2)",
                params![address, card.id.0],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn drop_card(&self, id: &CardId) -> Result<bool, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM contact_card_addresses WHERE card = ?1",
            params![id.0],
        )?;
        let gone = tx.execute("DELETE FROM contact_cards WHERE id = ?1", params![id.0])?;
        tx.commit()?;
        Ok(gone > 0)
    }
}
