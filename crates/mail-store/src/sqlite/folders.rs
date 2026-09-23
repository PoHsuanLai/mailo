//! Mailboxes: the listing, what a folder change does to every record keyed by a name, and
//! what a confirmed delete lets go of.

use super::SqliteStore;
use super::row::{json, to_json, uuid};
use crate::StoreError;
use mail_domain::folder::{layered, renamed};
use mail_domain::{
    AccountId, Change, Folder, FolderContents, FolderWork, LabelId, LabelOrigin, MailboxRef,
    MessageId, ProtoOp, ThreadId,
};
use rusqlite::params;
use std::collections::BTreeSet;

impl SqliteStore {
    /// Every folder held for `account`, by path.
    pub(super) fn read_folders(&self, account: AccountId) -> Result<Vec<Folder>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT path, delimiter, special, subscription, holds FROM folders
             WHERE account = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map(params![account.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (path, delimiter, special, subscription, holds) = row?;
            out.push(Folder {
                account,
                path,
                delimiter: delimiter.and_then(|d| d.chars().next()),
                special: special.map(|s| json("SpecialUse", &s)).transpose()?,
                subscription: json("Subscription", &subscription)?,
                holds: json("Holds", &holds)?,
            });
        }
        Ok(out)
    }

    /// Replace the listing with the server's, then lay queued folder work back on top.
    pub(super) fn write_folders(
        &self,
        account: AccountId,
        listed: Vec<Folder>,
    ) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let folders = layered(account, listed, &self.queued_folder_work(account)?);
        self.connection().execute(
            "DELETE FROM folders WHERE account = ?1",
            params![account.to_string()],
        )?;
        for folder in &folders {
            self.upsert_folder(folder)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Folder work still in the outbox, in the order it will be sent.
    fn queued_folder_work(&self, account: AccountId) -> Result<Vec<FolderWork>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached("SELECT op FROM outbox WHERE account = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![account.to_string()], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for text in rows {
            if let ProtoOp::Folder(work) = json::<ProtoOp>("ProtoOp", &text?)? {
                out.push(work);
            }
        }
        Ok(out)
    }

    pub(super) fn upsert_folder(&self, folder: &Folder) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO folders (account, path, delimiter, special, subscription, holds)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account, path) DO UPDATE SET
                 delimiter = excluded.delimiter, special = excluded.special,
                 subscription = excluded.subscription, holds = excluded.holds",
            params![
                folder.account.to_string(),
                folder.path,
                folder.delimiter.map(String::from),
                folder
                    .special
                    .map(|s| to_json("SpecialUse", &s))
                    .transpose()?,
                to_json("Subscription", &folder.subscription)?,
                to_json("Holds", &folder.holds)?,
            ],
        )?;
        Ok(())
    }

    pub(super) fn remove_folder(&self, mailbox: &MailboxRef) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM folders WHERE account = ?1 AND path = ?2",
            params![mailbox.account.to_string(), mailbox.path],
        )?;
        Ok(())
    }

    /// Rename `from` and everything beneath it, in every table that names a mailbox.
    ///
    /// Done in Rust rather than with `LIKE 'from/%'`: a path may contain `%` and `_`, which
    /// `LIKE` reads as wildcards, and the "beneath" rule is [`renamed`]'s to own, not a second
    /// spelling of it in SQL.
    pub(super) fn rename_folder(
        &self,
        from: &MailboxRef,
        to: &str,
        delimiter: Option<char>,
    ) -> Result<(), StoreError> {
        let account = from.account.to_string();
        for (table, column) in [
            ("folders", "path"),
            ("remote_map", "mailbox"),
            ("sync_state", "mailbox"),
        ] {
            let names: Vec<String> = {
                let db = self.connection();
                let mut stmt = db.prepare_cached(&format!(
                    "SELECT DISTINCT {column} FROM {table} WHERE account = ?1"
                ))?;
                let rows = stmt.query_map(params![account], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<_, _>>()?
            };
            for old in names {
                if let Some(new) = renamed(&old, &from.path, to, delimiter) {
                    self.connection().execute(
                        &format!(
                            "UPDATE {table} SET {column} = ?3 WHERE account = ?1 AND {column} = ?2"
                        ),
                        params![account, old, new],
                    )?;
                }
            }
        }
        // Where mailboxes are labels, the labels carry the same names. Only the server's: a
        // label the user made here that happens to share a name is theirs, and stays.
        let provider = to_json("LabelOrigin", &LabelOrigin::Provider)?;
        let labels: Vec<(String, String)> = {
            let db = self.connection();
            let mut stmt = db
                .prepare_cached("SELECT id, name FROM labels WHERE account = ?1 AND origin = ?2")?;
            let rows = stmt.query_map(params![account, provider], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        for (id, name) in labels {
            if let Some(new) = renamed(&name, &from.path, to, delimiter) {
                self.connection().execute(
                    "UPDATE labels SET name = ?2 WHERE id = ?1",
                    params![id, new],
                )?;
            }
        }
        Ok(())
    }

    /// Delete a label, rebuilding the summaries of any thread that still carried it.
    ///
    /// A planned delete takes the label off every message first, so normally there are none;
    /// an undo that removes a label the user has since filed mail under is the case this is for.
    pub(super) fn remove_label(&self, label: LabelId) -> Result<(), StoreError> {
        let threads: BTreeSet<ThreadId> = {
            let db = self.connection();
            let mut stmt = db.prepare_cached(
                "SELECT DISTINCT m.thread FROM message_labels l
                 JOIN messages m ON m.id = l.message WHERE l.label = ?1",
            )?;
            let rows = stmt.query_map(params![label.to_string()], |r| r.get::<_, String>(0))?;
            let mut out = BTreeSet::new();
            for row in rows {
                out.insert(ThreadId::from_uuid(uuid("ThreadId", &row?)?));
            }
            out
        };
        // `message_labels` goes with it, on the foreign key.
        self.connection().execute(
            "DELETE FROM labels WHERE id = ?1",
            params![label.to_string()],
        )?;
        for thread in threads {
            self.refresh_summary(thread)?;
        }
        Ok(())
    }

    /// What is held in one mailbox: messages addressed there, and messages carrying the
    /// server's label of that name.
    pub(super) fn contents(&self, mailbox: &MailboxRef) -> Result<FolderContents, StoreError> {
        let account = mailbox.account.to_string();
        let provider = to_json("LabelOrigin", &LabelOrigin::Provider)?;
        let db = self.connection();
        let mut mapped = db.prepare_cached(
            "SELECT DISTINCT message FROM remote_map WHERE account = ?1 AND mailbox = ?2",
        )?;
        let mapped = mapped
            .query_map(params![account, mailbox.path], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut labelled = db.prepare_cached(
            "SELECT DISTINCT l.message FROM message_labels l JOIN labels b ON b.id = l.label
             WHERE b.account = ?1 AND b.name = ?2 AND b.origin = ?3",
        )?;
        let labelled = labelled
            .query_map(params![account, mailbox.path, provider], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FolderContents {
            mapped: message_ids(mapped)?,
            labelled: message_ids(labelled)?,
        })
    }

    /// The server has confirmed a mailbox is gone: let go of its addresses and cursor, and of
    /// any message that was nowhere else.
    ///
    /// Only now, not when the delete was queued. An undo can put a folder back but not the
    /// addresses of what was in it, so dropping them early would strand every message the
    /// server went on holding after a refusal.
    ///
    /// On a server where mailboxes are labels, every message is still in All Mail, so nothing
    /// here is deleted: the addresses in the label's mailbox go, and the messages stay.
    pub(super) fn forget_mailbox(&self, account: AccountId, path: &str) -> Result<(), StoreError> {
        let held = self.contents(&MailboxRef {
            account,
            path: path.to_owned(),
        })?;
        self.connection().execute(
            "DELETE FROM remote_map WHERE account = ?1 AND mailbox = ?2",
            params![account.to_string(), path],
        )?;
        self.connection().execute(
            "DELETE FROM sync_state WHERE account = ?1 AND mailbox = ?2",
            params![account.to_string(), path],
        )?;
        for message in held.mapped {
            let remaining: i64 = self.connection().query_row(
                "SELECT count(*) FROM remote_map WHERE message = ?1",
                params![message.to_string()],
                |r| r.get(0),
            )?;
            if remaining == 0
                && let Some(thread) = self.write_change(&Change::MessageDelete(message))?
            {
                self.refresh_summary(thread)?;
            }
        }
        Ok(())
    }
}

/// Message ids from their stored text, sorted, so both stores answer in one order.
fn message_ids(texts: Vec<String>) -> Result<Vec<MessageId>, StoreError> {
    let mut out = texts
        .iter()
        .map(|t| uuid("MessageId", t).map(MessageId::from_uuid))
        .collect::<Result<Vec<_>, _>>()?;
    out.sort();
    Ok(out)
}
