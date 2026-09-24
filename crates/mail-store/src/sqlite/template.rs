//! Templates: messages kept to start new ones from. Local rows, never on a server.
//!
//! Beside [`super::draft`] and shaped like it, because a template carries a draft's fields in
//! the same encodings; apart from it because a template is never sent and has no state to move.

use super::SqliteStore;
use super::read::Recipients;
use super::row::{from_time, json, time, to_json, uuid};
use crate::StoreError;
use mail_domain::{AccountId, IdentityId, PendingAttachment, Template, TemplateId};
use rusqlite::params;

/// The columns a [`Template`] is read back from, in the order [`SqliteStore::read_template`]
/// expects.
const TEMPLATE_COLUMNS: &str = "id, account, identity, name, recipients, subject, body_text, \
     body_html, attachments, receipt, updated_at, openpgp, smime";

impl SqliteStore {
    pub(super) fn write_template(&self, template: &Template) -> Result<(), StoreError> {
        let recipients = Recipients {
            reply_to: Vec::new(),
            to: template.to.clone(),
            cc: template.cc.clone(),
            bcc: template.bcc.clone(),
        };
        self.connection().execute(
            "INSERT OR REPLACE INTO templates
               (id, account, identity, name, recipients, subject, body_text, body_html,
                attachments, receipt, updated_at, openpgp, smime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                template.id.to_string(),
                template.account.to_string(),
                template.identity.to_string(),
                template.name,
                to_json("Template.recipients", &recipients)?,
                template.subject,
                template.text,
                template.html,
                to_json("Template.attachments", &template.attachments)?,
                to_json("Template.receipt", &template.receipt)?,
                from_time(template.updated),
                to_json("Template.openpgp", &template.openpgp)?,
                to_json("Template.smime", &template.smime)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn remove_template(&self, id: TemplateId) -> Result<(), StoreError> {
        let gone = self.connection().execute(
            "DELETE FROM templates WHERE id = ?1",
            params![id.to_string()],
        )?;
        if gone == 0 {
            return Err(StoreError::NoTemplate(id));
        }
        Ok(())
    }

    pub(super) fn load_template(&self, id: TemplateId) -> Result<Template, StoreError> {
        let sql = format!("SELECT {TEMPLATE_COLUMNS} FROM templates WHERE id = ?1");
        let db = self.connection();
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![id.to_string()])?;
        match rows.next()? {
            Some(row) => Self::read_template(row),
            None => Err(StoreError::NoTemplate(id)),
        }
    }

    pub(super) fn load_templates(&self, account: AccountId) -> Result<Vec<Template>, StoreError> {
        // `NOCASE` folds ASCII only, which is what the in-memory store's ordering does too.
        let sql = format!(
            "SELECT {TEMPLATE_COLUMNS} FROM templates WHERE account = ?1
             ORDER BY name COLLATE NOCASE, id"
        );
        let db = self.connection();
        let mut stmt = db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![account.to_string()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::read_template(row)?);
        }
        Ok(out)
    }

    fn read_template(row: &rusqlite::Row<'_>) -> Result<Template, StoreError> {
        let recipients: Recipients = json("Template.recipients", &row.get::<_, String>(4)?)?;
        let attachments: Vec<PendingAttachment> =
            json("Template.attachments", &row.get::<_, String>(8)?)?;
        Ok(Template {
            id: TemplateId::from_uuid(uuid("TemplateId", &row.get::<_, String>(0)?)?),
            account: AccountId::from_uuid(uuid("AccountId", &row.get::<_, String>(1)?)?),
            identity: IdentityId::from_uuid(uuid("IdentityId", &row.get::<_, String>(2)?)?),
            name: row.get(3)?,
            to: recipients.to,
            cc: recipients.cc,
            bcc: recipients.bcc,
            subject: row.get(5)?,
            text: row.get(6)?,
            html: row.get(7)?,
            attachments,
            receipt: json("Template.receipt", &row.get::<_, String>(9)?)?,
            openpgp: json("Template.openpgp", &row.get::<_, String>(11)?)?,
            smime: json("Template.smime", &row.get::<_, String>(12)?)?,
            updated: time("Template.updated_at", &row.get::<_, String>(10)?)?,
        })
    }
}
