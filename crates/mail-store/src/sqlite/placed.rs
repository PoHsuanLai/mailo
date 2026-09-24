//! Where a message is held on the server, and whether it is still filed there here: the facts
//! [`mail_domain::Filter::InFolder`] weighs, one message at a time.
//!
//! The SQL compiler asks the same three questions inside a query (`sql::IN_FOLDER`); this is
//! the per-message form, for a caller that has a message rather than a filter to run.

use super::SqliteStore;
use crate::{Store, StoreError};
use mail_domain::{MailboxRef, MessageId, Placed};
use rusqlite::params;

impl SqliteStore {
    pub(super) fn load_placed(&self, message: MessageId) -> Result<Vec<Placed>, StoreError> {
        let held = self.message(message)?;
        let roles = self.folder_roles(held.account)?;
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT DISTINCT mailbox FROM remote_map WHERE message = ?1 ORDER BY mailbox",
        )?;
        let paths: Vec<String> = stmt
            .query_map(params![message.to_string()], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut stmt = db.prepare_cached(
            "SELECT json_extract(o.op, '$.v.folder') FROM pending_changes p
             JOIN outbox o ON o.id = p.outbox
             WHERE p.message = ?1 AND json_extract(o.op, '$.kind') = 'file'",
        )?;
        let moving: Vec<String> = stmt
            .query_map(params![message.to_string()], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let addresses = paths
            .into_iter()
            .map(|path| MailboxRef {
                account: held.account,
                path,
            })
            .collect();
        Ok(Placed::of_message(held.mailbox, addresses, &roles, &moving))
    }
}
