//! What the user answered a message's read-receipt request.
//!
//! Its own table, not a column on `messages`: an ingest rewrites the message row from server
//! truth, and the answer is not something the server's copy of the message knows.

use super::SqliteStore;
use super::row::{from_time, json, to_json};
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{MessageId, ReceiptAnswer};
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    pub(super) fn load_receipt_answer(
        &self,
        message: MessageId,
    ) -> Result<Option<ReceiptAnswer>, StoreError> {
        let db = self.reader();
        let text: Option<String> = db
            .query_row(
                "SELECT answer FROM receipt_answers WHERE message = ?1",
                params![message.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|t| json("ReceiptAnswer", &t)).transpose()
    }

    pub(super) fn write_receipt_answer(
        &self,
        message: MessageId,
        answer: ReceiptAnswer,
        now: DateTime<Utc>,
    ) -> Result<ReceiptAnswer, StoreError> {
        let db = self.connection();
        let known: Option<i64> = db
            .query_row(
                "SELECT 1 FROM messages WHERE id = ?1",
                params![message.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if known.is_none() {
            return Err(StoreError::NoMessage(message));
        }
        // `OR IGNORE`: the first answer stands. A message is asked once, and a second answer —
        // a receipt sent after it was declined — is exactly what recording the first prevents.
        db.execute(
            "INSERT OR IGNORE INTO receipt_answers (message, answer, answered_at)
             VALUES (?1, ?2, ?3)",
            params![
                message.to_string(),
                to_json("ReceiptAnswer", &answer)?,
                from_time(now)
            ],
        )?;
        let stored: String = db.query_row(
            "SELECT answer FROM receipt_answers WHERE message = ?1",
            params![message.to_string()],
            |r| r.get(0),
        )?;
        json("ReceiptAnswer", &stored)
    }
}
