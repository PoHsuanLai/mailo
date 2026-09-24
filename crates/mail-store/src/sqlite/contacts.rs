//! The address book in SQLite. The rules are `crate::contact::learn`'s; this is rows.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json, uuid};
use crate::StoreError;
use crate::contact::learn::{self, Event, NameRank, Needle, Row, Seen};
use crate::contact::{AddressBook, Contact, Kind, Origin, Tally};
use mail_domain::{
    AccountId, Address, BlobId, Draft, DraftId, MailboxRole, Message, MessageId, SendState,
};
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
        let db = self.connection();
        // Asked first, so a body arriving for a message already counted costs one insert and
        // not a read of its bytes.
        if !first_sighting(&db, id, message.mailbox)? {
            return Ok(());
        }
        let seen = Seen {
            mailbox: message.mailbox,
            date: message.date,
            from: &message.from,
            to: &message.to,
            cc: &message.cc,
            bcc: &message.bcc,
            subject: &message.subject,
            sender: self.sender_of(&db, message.mailbox, raw),
        };
        learn_seen(&db, account, &seen)
    }

    /// Whether a received message's raw bytes, where held, say it came through a list.
    fn sender_of(&self, db: &Connection, mailbox: MailboxRole, raw: Option<BlobId>) -> Kind {
        match (mailbox, raw) {
            (MailboxRole::Inbox | MailboxRole::Archive | MailboxRole::Trash, Some(raw)) => {
                self.sender_kind(db, raw)
            }
            _ => Kind::Person,
        }
    }

    /// Whether the raw message's headers say it came through a list. A blob that cannot be read
    /// says nothing, rather than stopping an ingest over a hint.
    fn sender_kind(&self, db: &Connection, raw: BlobId) -> Kind {
        match self.blobs.head(db, raw, HEAD) {
            Ok(head) if learn::listed(&head) => Kind::Bulk,
            _ => Kind::Person,
        }
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
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        let draft = self.load_draft(draft).ok();
        let db = self.connection();
        learn_sent(&db, account, draft.as_ref(), mail_from, rcpt_to, at)
    }

    /// Read every message already held through the book once, if migration 0013 asked for it.
    ///
    /// Sent drafts first, so a Sent-folder copy of one is recognised when the messages follow;
    /// then messages oldest first, the order they would have arrived in. A row that no longer
    /// decodes is passed over rather than stopping the database from opening.
    pub(super) fn backfill_contacts(&self) -> Result<(), StoreError> {
        let db = self.connection();
        let pending: i64 = db.query_row("SELECT count(*) FROM contacts_to_backfill", [], |r| {
            r.get(0)
        })?;
        if pending == 0 {
            return Ok(());
        }
        let tx = db.unchecked_transaction()?;

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
            learn_sent(&db, draft.account, Some(&draft), &from, &[], at)?;
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
                    AccountId::from_uuid(uuid("messages.account", &account)?),
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
            if !first_sighting(&db, id, mailbox)? {
                continue;
            }
            let sender = self.sender_of(&db, mailbox, raw);
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
            learn_seen(&db, account, &seen)?;
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
        let db = self.connection();
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
            .connection()
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
        self.connection().execute(
            "INSERT INTO address_books (url, state) VALUES (?1, ?2)
             ON CONFLICT(url) DO UPDATE SET state = excluded.state",
            params![book.url, to_json("AddressBook", book)?],
        )?;
        Ok(())
    }
}

/// Whether message `id` is one the book has yet to count, marking it counted if so. Spam and
/// drafts are never counted, and never marked.
fn first_sighting(
    db: &Connection,
    id: MessageId,
    mailbox: MailboxRole,
) -> Result<bool, StoreError> {
    if matches!(mailbox, MailboxRole::Spam | MailboxRole::Drafts) {
        return Ok(false);
    }
    let first = db.execute(
        "INSERT OR IGNORE INTO contacts_counted (message) VALUES (?1)",
        params![id.to_string()],
    )?;
    Ok(first > 0)
}

/// Count a message seen for the first time, recognising the Sent copy of a submission already
/// counted.
fn learn_seen(db: &Connection, account: AccountId, seen: &Seen<'_>) -> Result<(), StoreError> {
    let mut events = learn::events(seen);
    if seen.mailbox == MailboxRole::Sent {
        let print = learn::fingerprint([seen.to, seen.cc], seen.subject);
        let counted = db.execute(
            "DELETE FROM contacts_sent WHERE rowid =
                 (SELECT min(rowid) FROM contacts_sent WHERE fingerprint = ?1)",
            params![print],
        )?;
        if counted > 0 {
            events.retain(|(_, e)| *e == Event::Own);
        }
    }
    hear(db, Some(account), &events)
}

/// Count a confirmed submission: from the draft where it still exists, else from the envelope.
fn learn_sent(
    db: &Connection,
    account: AccountId,
    draft: Option<&Draft>,
    mail_from: &str,
    rcpt_to: &[String],
    at: chrono::DateTime<chrono::Utc>,
) -> Result<(), StoreError> {
    let envelope: Vec<Address> = rcpt_to
        .iter()
        .map(|email| Address {
            name: None,
            email: email.clone(),
        })
        .collect();
    let (to, cc, bcc, subject) = match draft {
        Some(d) => (&d.to[..], &d.cc[..], &d.bcc[..], d.subject.as_str()),
        None => (&envelope[..], &[][..], &[][..], ""),
    };
    let from = Address {
        name: None,
        email: mail_from.to_owned(),
    };
    db.execute(
        "INSERT INTO contacts_sent (fingerprint, at) VALUES (?1, ?2)",
        params![learn::fingerprint([to, cc], subject), from_time(at)],
    )?;
    hear(db, Some(account), &learn::sent(&from, [to, cc, bcc], at))
}

fn hear(
    db: &Connection,
    account: Option<AccountId>,
    events: &[(String, Event)],
) -> Result<(), StoreError> {
    for (address, event) in events {
        let row = load(db, address)?.unwrap_or_else(|| Row::new(address));
        save(db, &learn::apply(row, account, event))?;
    }
    Ok(())
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
        c.account.map(|a| a.to_string()),
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
                .map(|a| uuid("contacts.account", &a).map(AccountId::from_uuid))
                .transpose()?,
        },
        name_rank: NameRank::from_i64(r.get(2)?),
        name_at: when(3, "contacts.name_at")?,
        score: r.get(8)?,
    })
}
