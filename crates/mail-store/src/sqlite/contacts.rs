//! The address book in SQLite. The rules are `crate::contact::learn`'s; this is rows.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json, uuid};
use crate::StoreError;
use crate::changes::Topics;
use crate::contact::learn::{self, Event, NameRank, Needle, Row, Seen};
use crate::contact::ledger::{self, Ledger};
use crate::contact::{AddressBook, Contact, Origin, Tally};
use chrono::{DateTime, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{Address, BlobId, DraftId, MailboxRole, Message, MessageId, SendState};
use porter_core::AccountId;
use rusqlite::{Connection, OptionalExtension, params};

const COLUMNS: &str = "address, name, name_rank, name_at, written_count, written_last, \
     received_count, received_last, score, kind, origin, account";

/// How much of a raw message to read for its list headers. Header sections past this are rare
/// enough, and a missed `List-Id` cheap enough, not to read whole attachments to find one.
const HEAD: usize = 16 * 1024;

impl SqliteStore {
    /// What one ingested message teaches the book, once per message.
    pub(super) fn learn_fetched(
        &self,
        account: AccountId,
        id: MessageId,
        message: &Message,
        raw: Option<BlobId>,
    ) -> Result<(), StoreError> {
        let db = self.touch(Topics::CONTACTS);
        ledger::learn_fetched(
            &mut Rows {
                store: self,
                db: &db,
            },
            account,
            id,
            message,
            raw,
        )
    }

    /// What a submission the outbox just confirmed teaches the book: the draft's recipients were
    /// written to, under the names the user gave them. Recorded so its Sent copy is not counted
    /// again when it arrives.
    pub(super) fn learn_submission(
        &self,
        account: AccountId,
        draft: DraftId,
        mail_from: &str,
        rcpt_to: &[String],
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // A draft gone since it was sent has nothing more to teach; any other failure is one.
        let draft = match self.load_draft(draft) {
            Ok(draft) => Some(draft),
            Err(StoreError::NoDraft(_)) => None,
            Err(error) => return Err(error),
        };
        let db = self.touch(Topics::CONTACTS);
        let mut held = Rows {
            store: self,
            db: &db,
        };
        ledger::learn_sent(&mut held, account, draft.as_ref(), mail_from, rcpt_to, at)
    }

    /// Read every message already held through the book once, if migration 0013 asked for it.
    ///
    /// Sent drafts first, so a Sent-folder copy of one is recognised when the messages follow;
    /// then messages oldest first, the order they would have arrived in. A row that no longer
    /// decodes is passed over rather than stopping the database from opening.
    pub(super) fn backfill_contacts(&self) -> Result<(), StoreError> {
        let db = self.touch(Topics::CONTACTS);
        let pending: i64 = db.query_row("SELECT count(*) FROM contacts_to_backfill", [], |r| {
            r.get(0)
        })?;
        if pending == 0 {
            return Ok(());
        }
        let tx = db.unchecked_transaction()?;
        let mut held = Rows {
            store: self,
            db: &db,
        };

        let drafts: Vec<String> = {
            let mut stmt = db.prepare("SELECT id FROM drafts ORDER BY updated_at")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for id in drafts {
            let Ok(id) = uuid("drafts.id", &id).map(DraftId::from_uuid) else {
                continue;
            };
            let Ok(draft) = self.load_draft(id) else {
                continue;
            };
            let SendState::Sent { at, .. } = draft.state else {
                continue;
            };
            let from: Option<String> = db
                .query_row(
                    "SELECT from_email FROM identities WHERE id = ?1",
                    params![draft.identity.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let from = from.unwrap_or_default();
            let account = draft.account.clone();
            ledger::learn_sent(&mut held, account, Some(&draft), &from, &[], at)?;
        }

        type Held = (
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
        );
        let messages: Vec<Held> = {
            let mut stmt = db.prepare(
                "SELECT id, account, mailbox, date, from_name, from_email, recipients, subject,
                        body_raw
                 FROM messages ORDER BY date, id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        for (id, account, mailbox, date, from_name, from_email, recipients, subject, raw) in
            messages
        {
            let decoded = (|| -> Result<_, StoreError> {
                Ok((
                    MessageId::from_uuid(uuid("messages.id", &id)?),
                    account_id_from_uuid(uuid("messages.account", &account)?),
                    json::<MailboxRole>("messages.mailbox", &mailbox)?,
                    time("messages.date", &date)?,
                    json::<super::read::Recipients>("messages.recipients", &recipients)?,
                ))
            })();
            let Ok((id, account, mailbox, date, recipients)) = decoded else {
                continue;
            };
            let raw = raw
                .and_then(|r| uuid("messages.body_raw", &r).ok())
                .map(BlobId::from_uuid);
            if !ledger::first_sighting(&mut held, id, mailbox)? {
                continue;
            }
            let sender = ledger::sender_of(&held, mailbox, raw);
            let from = Address {
                name: from_name,
                email: from_email,
            };
            let seen = Seen {
                mailbox,
                date,
                from: &from,
                to: &recipients.to,
                cc: &recipients.cc,
                bcc: &recipients.bcc,
                subject: &subject,
                sender,
            };
            ledger::learn_seen(&mut held, account, &seen)?;
        }

        tx.execute("DELETE FROM contacts_to_backfill", [])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn contacts_like(&self, typed: &str, k: usize) -> Result<Vec<Contact>, StoreError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let needle = Needle::new(typed);
        let db = self.reader();
        let order = "ORDER BY score DESC NULLS LAST, address";
        let mut out = Vec::new();
        let mut take = |row: Row, keys: &str| {
            if needle.matches(keys) && row.contact.offered() {
                out.push(row.contact);
            }
            out.len() < k
        };
        match needle.narrow() {
            None => {
                let mut stmt =
                    db.prepare_cached(&format!("SELECT {COLUMNS}, keys FROM contacts {order}"))?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    if !take(read_row(r)?, &r.get::<_, String>(12)?) {
                        break;
                    }
                }
            }
            Some((raw, word)) => {
                let mut stmt = db.prepare_cached(&format!(
                    "SELECT {COLUMNS}, keys FROM contacts
                     WHERE instr(keys, ' ' || ?1) > 0 OR instr(keys, ' ' || ?2) > 0 {order}"
                ))?;
                let mut rows = stmt.query(params![raw, word])?;
                while let Some(r) = rows.next()? {
                    if !take(read_row(r)?, &r.get::<_, String>(12)?) {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }

    pub(super) fn every_contact(&self) -> Result<Vec<Contact>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM contacts ORDER BY score DESC NULLS LAST, address"
        ))?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(read_row(r)?.contact);
        }
        Ok(out)
    }

    pub(super) fn one_contact(&self, address: &str) -> Result<Option<Contact>, StoreError> {
        let Some(address) = learn::normalise(address) else {
            return Ok(None);
        };
        Ok(load(&self.reader(), &address)?.map(|row| row.contact))
    }

    pub(super) fn give_contact(
        &self,
        address: &str,
        name: Option<&str>,
        origin: &Origin,
    ) -> Result<Contact, StoreError> {
        let address =
            learn::normalise(address).ok_or_else(|| StoreError::BadAddress(address.to_owned()))?;
        let db = self.touch(Topics::CONTACTS);
        let row = load(&db, &address)?.unwrap_or_else(|| Row::new(&address));
        let row = learn::given(row, name, origin);
        save(&db, &row)?;
        Ok(row.contact)
    }

    pub(super) fn drop_contact(&self, address: &str) -> Result<bool, StoreError> {
        let Some(address) = learn::normalise(address) else {
            return Ok(false);
        };
        let gone = self
            .touch(Topics::CONTACTS)
            .execute("DELETE FROM contacts WHERE address = ?1", params![address])?;
        Ok(gone > 0)
    }

    pub(super) fn read_book(&self, url: &str) -> Result<Option<AddressBook>, StoreError> {
        let state: Option<String> = self
            .reader()
            .query_row(
                "SELECT state FROM address_books WHERE url = ?1",
                params![url],
                |r| r.get(0),
            )
            .optional()?;
        state.map(|s| json("AddressBook", &s)).transpose()
    }

    pub(super) fn every_book(&self) -> Result<Vec<AddressBook>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached("SELECT state FROM address_books ORDER BY url")?;
        let states = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for state in states {
            out.push(json("AddressBook", &state?)?);
        }
        Ok(out)
    }

    pub(super) fn write_book(&self, book: &AddressBook) -> Result<(), StoreError> {
        self.touch(Topics::CONTACTS).execute(
            "INSERT INTO address_books (url, state) VALUES (?1, ?2)
             ON CONFLICT(url) DO UPDATE SET state = excluded.state",
            params![book.url, to_json("AddressBook", book)?],
        )?;
        Ok(())
    }
}

/// The SQLite rows the learning policy runs over, on a connection already held.
struct Rows<'a> {
    store: &'a SqliteStore,
    db: &'a Connection,
}

impl Ledger for Rows<'_> {
    fn first_count(&mut self, id: MessageId) -> Result<bool, StoreError> {
        let first = self
            .db
            .prepare_cached("INSERT OR IGNORE INTO contacts_counted (message) VALUES (?1)")?
            .execute(params![id.to_string()])?;
        Ok(first > 0)
    }

    fn head(&self, raw: BlobId) -> Option<Vec<u8>> {
        // A blob that cannot be read says nothing, rather than stopping an ingest over a hint.
        self.store.blobs.head(self.db, raw, HEAD).ok()
    }

    fn take_print(&mut self, print: &str) -> Result<bool, StoreError> {
        let counted = self
            .db
            .prepare_cached(
                "DELETE FROM contacts_sent WHERE rowid =
                 (SELECT min(rowid) FROM contacts_sent WHERE fingerprint = ?1)",
            )?
            .execute(params![print])?;
        Ok(counted > 0)
    }

    fn keep_print(&mut self, print: &str, at: DateTime<Utc>) -> Result<(), StoreError> {
        self.db
            .prepare_cached("INSERT INTO contacts_sent (fingerprint, at) VALUES (?1, ?2)")?
            .execute(params![print, from_time(at)])?;
        Ok(())
    }

    fn hear(
        &mut self,
        account: Option<AccountId>,
        events: &[(String, Event)],
    ) -> Result<(), StoreError> {
        for (address, event) in events {
            let row = load(self.db, address)?.unwrap_or_else(|| Row::new(address));
            save(self.db, &learn::apply(row, account.clone(), event))?;
        }
        Ok(())
    }
}

fn load(db: &Connection, address: &str) -> Result<Option<Row>, StoreError> {
    let mut stmt = db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM contacts WHERE address = ?1"
    ))?;
    let mut rows = stmt.query(params![address])?;
    rows.next()?.map(read_row).transpose()
}

fn save(db: &Connection, row: &Row) -> Result<(), StoreError> {
    let c = &row.contact;
    db.prepare_cached(&format!(
        "INSERT OR REPLACE INTO contacts ({COLUMNS}, keys)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
    ))?
    .execute(params![
        c.address,
        c.name,
        row.name_rank as i64,
        row.name_at.map(from_time),
        i64::from(c.written.count),
        c.written.last.map(from_time),
        i64::from(c.received.count),
        c.received.last.map(from_time),
        row.score,
        to_json("Kind", &c.kind)?,
        to_json("Origin", &c.origin)?,
        c.account.clone().map(|a| a.to_string()),
        learn::keys(c),
    ])?;
    Ok(())
}

fn read_row(r: &rusqlite::Row<'_>) -> Result<Row, StoreError> {
    let when =
        |i: usize, what: &str| -> Result<Option<chrono::DateTime<chrono::Utc>>, StoreError> {
            r.get::<_, Option<String>>(i)?
                .map(|t| time(what, &t))
                .transpose()
        };
    let count = |i: usize| -> Result<u32, StoreError> {
        Ok(u32::try_from(r.get::<_, i64>(i)?.max(0)).unwrap_or(u32::MAX))
    };
    Ok(Row {
        contact: Contact {
            address: r.get(0)?,
            name: r.get(1)?,
            written: Tally {
                count: count(4)?,
                last: when(5, "contacts.written_last")?,
            },
            received: Tally {
                count: count(6)?,
                last: when(7, "contacts.received_last")?,
            },
            kind: json("Kind", &r.get::<_, String>(9)?)?,
            origin: json("Origin", &r.get::<_, String>(10)?)?,
            account: r
                .get::<_, Option<String>>(11)?
                .map(|a| uuid("contacts.account", &a).map(mail_domain::id::account_id_from_uuid))
                .transpose()?,
        },
        name_rank: NameRank::from_i64(r.get(2)?),
        name_at: when(3, "contacts.name_at")?,
        score: r.get(8)?,
    })
}
