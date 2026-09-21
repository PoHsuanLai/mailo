//! Drafts: the one object that is born local and may never have a server side at all.
//!
//! Separate from [`super::write`] because a draft is not part of the patch/ingest duality that
//! module is about. A draft has no thread, no remote address and no flags to reconcile; it is
//! simply a row the user owns until they send it.

use super::SqliteStore;
use super::read::Recipients;
use super::row::{from_time, json, time, to_json, uuid};
use crate::StoreError;
use mail_domain::{AccountId, Draft, DraftId, MessageId, PendingAttachment, SendState};
use rusqlite::params;

/// The columns a [`Draft`] is read back from, in the order [`SqliteStore::read_draft`] expects.
const DRAFT_COLUMNS: &str = "id, account, identity, recipients, subject, in_reply_to, \
     forward_of, body_text, body_html, attachments, state, updated_at";

impl SqliteStore {
    /// Insert or replace a draft.
    ///
    /// `INSERT OR REPLACE` rather than an `UPDATE`, because the caller does not know whether
    /// this draft has been persisted before: a composer autosaving every keystroke would
    /// otherwise have to ask first, and the answer would be stale by the time it acted on it.
    pub(super) fn write_draft(&self, draft: &Draft) -> Result<(), StoreError> {
        let recipients = Recipients {
            // A draft has no `Reply-To` of its own: it comes from the identity at build time,
            // which is where `mail_mime::build` reads it from.
            reply_to: Vec::new(),
            to: draft.to.clone(),
            cc: draft.cc.clone(),
            bcc: draft.bcc.clone(),
        };
        self.connection().execute(
            "INSERT OR REPLACE INTO drafts
               (id, account, identity, recipients, subject, in_reply_to, forward_of,
                body_text, body_html, attachments, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                draft.id.to_string(),
                draft.account.to_string(),
                draft.identity.to_string(),
                to_json("Draft.recipients", &recipients)?,
                draft.subject,
                draft.in_reply_to.map(|m| m.to_string()),
                draft.forward_of.map(|m| m.to_string()),
                draft.text,
                draft.html,
                to_json("Draft.attachments", &draft.attachments)?,
                to_json("SendState", &draft.state)?,
                from_time(draft.updated),
            ],
        )?;
        Ok(())
    }

    pub(super) fn delete_draft(&self, id: DraftId) -> Result<(), StoreError> {
        self.connection()
            .execute("DELETE FROM drafts WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    /// One draft by id.
    pub(super) fn load_draft(&self, id: DraftId) -> Result<Draft, StoreError> {
        let sql = format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1");
        let db = self.connection();
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![id.to_string()])?;
        match rows.next()? {
            Some(row) => Self::read_draft(row),
            None => Err(StoreError::NoDraft(id)),
        }
    }

    /// Every draft on an account, most recently touched first.
    ///
    /// Newest first because the only list a composer shows is "what was I working on", and
    /// that is answered by recency, not by creation order.
    pub(super) fn load_drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError> {
        let sql = format!(
            "SELECT {DRAFT_COLUMNS} FROM drafts WHERE account = ?1 ORDER BY updated_at DESC"
        );
        let db = self.connection();
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![account.to_string()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::read_draft(row)?);
        }
        Ok(out)
    }

    fn read_draft(row: &rusqlite::Row<'_>) -> Result<Draft, StoreError> {
        let recipients: Recipients = json("Draft.recipients", &row.get::<_, String>(3)?)?;
        let attachments: Vec<PendingAttachment> =
            json("Draft.attachments", &row.get::<_, String>(9)?)?;
        let state: SendState = json("SendState", &row.get::<_, String>(10)?)?;
        let message = |text: Option<String>, what: &str| -> Result<Option<MessageId>, StoreError> {
            text.map(|t| uuid(what, &t).map(MessageId::from_uuid))
                .transpose()
        };
        Ok(Draft {
            id: DraftId::from_uuid(uuid("DraftId", &row.get::<_, String>(0)?)?),
            account: AccountId::from_uuid(uuid("AccountId", &row.get::<_, String>(1)?)?),
            identity: mail_domain::IdentityId::from_uuid(uuid(
                "IdentityId",
                &row.get::<_, String>(2)?,
            )?),
            to: recipients.to,
            cc: recipients.cc,
            bcc: recipients.bcc,
            subject: row.get(4)?,
            in_reply_to: message(row.get(5)?, "Draft.in_reply_to")?,
            forward_of: message(row.get(6)?, "Draft.forward_of")?,
            text: row.get(7)?,
            html: row.get(8)?,
            attachments,
            state,
            updated: time("Draft.updated_at", &row.get::<_, String>(11)?)?,
        })
    }

    /// Move a draft to a new [`SendState`] without rewriting the rest of it.
    ///
    /// Narrow on purpose: the send path changes only this column, and a full upsert from a
    /// value the sender has been holding would quietly revert an edit made while it was in
    /// flight.
    pub(super) fn set_send_state(
        &self,
        id: DraftId,
        state: &SendState,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        let changed = self.connection().execute(
            "UPDATE drafts SET state = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), to_json("SendState", state)?, from_time(now)],
        )?;
        if changed == 0 {
            return Err(StoreError::NoDraft(id));
        }
        Ok(())
    }
}
