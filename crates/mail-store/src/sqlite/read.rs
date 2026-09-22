//! Reading threads and messages back out.

use super::SqliteStore;
use super::row::{json, time, uuid};
use crate::StoreError;
use mail_domain::{
    Address, Attachment, Attachments, BlobId, Body, LabelId, MailboxRole, MailboxSet, Message,
    MessageId, MessageKey, Pin, ReadState, Snooze, Star, Thread, ThreadId, ThreadSummary,
};
use rusqlite::{Connection, Row, params};

/// The `to`/`cc`/`bcc`/`reply_to` bundle, stored as one JSON column.
#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct Recipients {
    #[serde(default)]
    pub reply_to: Vec<Address>,
    #[serde(default)]
    pub to: Vec<Address>,
    #[serde(default)]
    pub cc: Vec<Address>,
    #[serde(default)]
    pub bcc: Vec<Address>,
}

pub const MESSAGE_COLUMNS: &str = "id, thread, account, msg_key, date, from_name, from_email, \
     recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox, body_text, \
     body_raw, attachments";

pub const SUMMARY_COLUMNS: &str = "thread, account, subject, snippet, from_name, from_email, \
     participants, recipients, last_date, message_count, read, star, mailboxes, labels, \
     attachments, snooze, pin";

impl SqliteStore {
    /// `db` is the connection the row came from, so the labels are read from the same world.
    pub(super) fn read_message(
        &self,
        db: &Connection,
        row: &Row<'_>,
    ) -> Result<Message, StoreError> {
        let recipients: Recipients = json("Message.recipients", &row.get::<_, String>(7)?)?;
        let id = MessageId::from_uuid(uuid("MessageId", &row.get::<_, String>(0)?)?);
        let labels = self.labels_of(db, id)?;
        Ok(Message {
            id,
            thread: ThreadId::from_uuid(uuid("ThreadId", &row.get::<_, String>(1)?)?),
            account: mail_domain::AccountId::from_uuid(uuid(
                "AccountId",
                &row.get::<_, String>(2)?,
            )?),
            key: json::<MessageKey>("MessageKey", &row.get::<_, String>(3)?)?,
            date: time("Message.date", &row.get::<_, String>(4)?)?,
            from: Address {
                name: row.get::<_, Option<String>>(5)?,
                email: row.get::<_, String>(6)?,
            },
            reply_to: recipients.reply_to,
            to: recipients.to,
            cc: recipients.cc,
            bcc: recipients.bcc,
            subject: row.get::<_, String>(8)?,
            in_reply_to: row.get::<_, Option<String>>(9)?,
            references: json("Message.references", &row.get::<_, String>(10)?)?,
            rfc_message_id: row.get::<_, Option<String>>(11)?,
            read: json("ReadState", &row.get::<_, String>(12)?)?,
            star: json("Star", &row.get::<_, String>(13)?)?,
            mailbox: json::<MailboxRole>("MailboxRole", &row.get::<_, String>(14)?)?,
            labels,
            // A NULL body_raw is headers-only, which is a normal state after a POP3 TOP
            // pass or an IMAP envelope fetch — not a corrupt row.
            body: match row.get::<_, Option<String>>(16)? {
                Some(raw) => Body::Present {
                    text: row.get::<_, Option<String>>(15)?,
                    raw: BlobId::from_uuid(uuid("BlobId", &raw)?),
                },
                None => Body::Absent,
            },
            attachments: json::<Vec<Attachment>>("attachments", &row.get::<_, String>(17)?)?,
        })
    }

    pub(super) fn read_summary(&self, row: &Row<'_>) -> Result<ThreadSummary, StoreError> {
        Ok(ThreadSummary {
            id: ThreadId::from_uuid(uuid("ThreadId", &row.get::<_, String>(0)?)?),
            account: mail_domain::AccountId::from_uuid(uuid(
                "AccountId",
                &row.get::<_, String>(1)?,
            )?),
            subject: row.get::<_, String>(2)?,
            snippet: row.get::<_, String>(3)?,
            from: Address {
                name: row.get::<_, Option<String>>(4)?,
                email: row.get::<_, String>(5)?,
            },
            participants: json("participants", &row.get::<_, String>(6)?)?,
            recipients: json("recipients", &row.get::<_, String>(7)?)?,
            last_date: time("last_date", &row.get::<_, String>(8)?)?,
            message_count: row.get::<_, i64>(9)? as u32,
            read: json::<ReadState>("ReadState", &row.get::<_, String>(10)?)?,
            star: json::<Star>("Star", &row.get::<_, String>(11)?)?,
            mailboxes: json::<MailboxSet>("MailboxSet", &row.get::<_, String>(12)?)?,
            labels: json::<Vec<LabelId>>("labels", &row.get::<_, String>(13)?)?,
            attachments: json::<Attachments>("Attachments", &row.get::<_, String>(14)?)?,
            snooze: json::<Snooze>("Snooze", &row.get::<_, String>(15)?)?,
            pin: json::<Pin>("Pin", &row.get::<_, String>(16)?)?,
        })
    }

    /// Every label on this account, by name.
    ///
    /// For the search box, which needs a name to become a [`LabelId`], and for anything that
    /// wants to show the user what there is. Ordered by name so two calls agree.
    pub fn labels(
        &self,
        account: mail_domain::AccountId,
    ) -> Result<Vec<mail_domain::Label>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare_cached(
            "SELECT id, name, color, origin FROM labels WHERE account = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![account.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, color, origin) = row?;
            out.push(mail_domain::Label {
                id: uuid("LabelId", &id).map(LabelId::from_uuid)?,
                account,
                name,
                color,
                origin: json("LabelOrigin", &origin)?,
            });
        }
        Ok(out)
    }

    /// `db` rather than reaching for one: this is called both from a read and from inside
    /// `write_patch`'s transaction, and those are two different worlds. See `SqliteStore::reader`.
    pub(super) fn labels_of(
        &self,
        db: &Connection,
        message: MessageId,
    ) -> Result<Vec<LabelId>, StoreError> {
        let mut stmt = db
            .prepare_cached("SELECT label FROM message_labels WHERE message = ?1 ORDER BY label")?;
        let rows = stmt.query_map(params![message.to_string()], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for text in rows {
            out.push(LabelId::from_uuid(uuid("LabelId", &text?)?));
        }
        Ok(out)
    }

    /// Every message of a thread, oldest first.
    pub(super) fn messages_of(
        &self,
        db: &Connection,
        thread: ThreadId,
    ) -> Result<Vec<Message>, StoreError> {
        let sql =
            format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread = ?1 ORDER BY date, id");
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![thread.to_string()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(self.read_message(db, row)?);
        }
        Ok(out)
    }

    pub(super) fn summary_of(
        &self,
        db: &Connection,
        thread: ThreadId,
    ) -> Result<ThreadSummary, StoreError> {
        let sql = format!("SELECT {SUMMARY_COLUMNS} FROM thread_summary WHERE thread = ?1");
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![thread.to_string()])?;
        match rows.next()? {
            Some(row) => self.read_summary(row),
            None => Err(StoreError::NoThread(thread)),
        }
    }

    pub(super) fn load_thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        let db = self.reader();
        let summary = self.summary_of(&db, id)?;
        let messages = self
            .messages_of(&db, id)?
            .into_iter()
            .map(|m| m.id)
            .collect();
        Ok(Thread { summary, messages })
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Db(e.to_string())
    }
}
