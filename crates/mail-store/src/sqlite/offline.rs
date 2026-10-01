//! What of an account is held here, and which attachments wait on the server. Kept in step
//! with `memory/offline.rs`, which `tests/offline.rs` holds it to.
//!
//! Attachments are a JSON column (`serde(Vec<Attachment>)`), and a part left on the server is an
//! element with a `remote_section` and no `blob`. The `LIKE` is a cheap first cut before
//! `json_each` opens the column: most messages have no such element, and it takes the text of
//! the key being there for one to.

use super::SqliteStore;
use super::row::uuid;
use crate::{Offline, RemotePart, StoreError};
use mail_domain::{AccountId, MailboxRef, MessageId};
use rusqlite::params;

/// An element of `m.attachments`, as `a`, that waits on the server as a section it can be asked
/// for. The same test as `offline::is_remote`.
const REMOTE: &str = "json_extract(a.value, '$.blob') IS NULL \
     AND COALESCE(json_extract(a.value, '$.remote_section'), '') != ''";

/// Rows whose attachments could hold a remote part at all.
const MAY_HOLD: &str = "m.attachments LIKE '%\"remote_section\"%'";

impl SqliteStore {
    pub(super) fn read_remote_parts(
        &self,
        mailbox: &MailboxRef,
        limit: u32,
    ) -> Result<Vec<RemotePart>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(&format!(
            "SELECT m.id, json_extract(a.value, '$.remote_section'), json_extract(a.value, '$.size')
             FROM messages m, json_each(m.attachments) a
             WHERE m.account = ?1 AND {MAY_HOLD} AND {REMOTE}
               AND EXISTS (SELECT 1 FROM remote_map r
                           WHERE r.message = m.id AND r.account = ?1 AND r.mailbox = ?2)
             ORDER BY json_extract(a.value, '$.size') ASC, m.date DESC, m.id DESC,
                      json_extract(a.value, '$.remote_section') ASC
             LIMIT ?3"
        ))?;
        let rows = stmt.query_map(
            params![mailbox.account.to_string(), mailbox.path, i64::from(limit)],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (id, section, size) = row?;
            Ok(RemotePart {
                message: MessageId::from_uuid(uuid("RemotePart.message", &id)?),
                section,
                size: u64::try_from(size).unwrap_or(0),
            })
        })
        .collect()
    }

    pub(super) fn read_offline(&self, account: AccountId) -> Result<Offline, StoreError> {
        let db = self.connection();
        let account = account.to_string();
        let (messages, held): (i64, i64) = db.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(
                     m.body_raw IS NOT NULL AND NOT ({MAY_HOLD} AND EXISTS (
                         SELECT 1 FROM json_each(m.attachments) a WHERE {REMOTE}))), 0)
                 FROM messages m WHERE m.account = ?1"
            ),
            [&account],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (parts_remote, remote_bytes): (i64, i64) = db.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(json_extract(a.value, '$.size')), 0)
                 FROM messages m, json_each(m.attachments) a
                 WHERE m.account = ?1 AND {MAY_HOLD} AND {REMOTE}"
            ),
            [&account],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let count = |n: i64| u64::try_from(n).unwrap_or(0);
        Ok(Offline {
            messages: count(messages),
            held: count(held),
            parts_remote: count(parts_remote),
            remote_bytes: count(remote_bytes),
        })
    }
}
