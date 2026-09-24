//! What the user answered a calendar invitation.
//!
//! Its own table, like read-receipt answers and for the same reason: an ingest rewrites the
//! message row from server truth, and the answer is not in the server's copy of the message.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json};
use crate::StoreError;
use mail_domain::{InviteAnswer, MessageId};
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    pub(super) fn load_invite_answer(
        &self,
        message: MessageId,
    ) -> Result<Option<InviteAnswer>, StoreError> {
        let db = self.reader();
        let row: Option<(String, i64, Option<String>, String)> = db
            .query_row(
                "SELECT attendance, sequence, comment, answered_at
                 FROM invite_answers WHERE message = ?1",
                params![message.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(attendance, sequence, comment, answered_at)| {
            Ok(InviteAnswer {
                message,
                attendance: json("Attendance", &attendance)?,
                // Written from a `u32`, so only a hand-edited row is out of range.
                sequence: u32::try_from(sequence).unwrap_or(0),
                comment,
                answered_at: time("invite_answers.answered_at", &answered_at)?,
            })
        })
        .transpose()
    }

    pub(super) fn write_invite_answer(&self, answer: &InviteAnswer) -> Result<(), StoreError> {
        let db = self.connection();
        let known: Option<i64> = db
            .query_row(
                "SELECT 1 FROM messages WHERE id = ?1",
                params![answer.message.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if known.is_none() {
            return Err(StoreError::NoMessage(answer.message));
        }
        // Replaced, unlike a receipt answer: a person may change their mind about a meeting,
        // and each change is a new reply the organiser receives.
        db.execute(
            "INSERT OR REPLACE INTO invite_answers
                 (message, attendance, sequence, comment, answered_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                answer.message.to_string(),
                to_json("Attendance", &answer.attendance)?,
                i64::from(answer.sequence),
                answer.comment,
                from_time(answer.answered_at)
            ],
        )?;
        Ok(())
    }
}
