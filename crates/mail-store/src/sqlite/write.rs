//! Writing: local patches, and server truth.
//!
//! The two are deliberately different operations. A [`Patch`] is small, invertible and
//! optimistic; an [`Ingest`] is bulk, not invertible, and *is* the truth. What joins them is
//! the reconciliation rule in [`SqliteStore::write_ingest`].

use super::SqliteStore;
use super::read::Recipients;
use super::row::{from_time, json, to_json, uuid};
use crate::StoreError;
use mail_domain::{
    AccountId, Body, Change, Ingest, Membership, Message, MessageId, Patch, RemoteRef, ThreadId,
    ThreadSummary, UidValidity,
};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;

/// A `RemoteRef` flattened into the four columns `remote_map` is keyed by.
fn remote_key(
    account: AccountId,
    r: &RemoteRef,
) -> (String, String, Option<i64>, Option<i64>, Option<String>) {
    match r {
        RemoteRef::Imap {
            mailbox,
            uidvalidity,
            uid,
        } => (
            account.to_string(),
            mailbox.clone(),
            Some(i64::from(*uidvalidity)),
            Some(i64::from(*uid)),
            None,
        ),
        RemoteRef::Pop { uidl } => (
            account.to_string(),
            "INBOX".to_owned(),
            None,
            None,
            Some(uidl.clone()),
        ),
    }
}

impl SqliteStore {
    /// Apply one domain change. Does not recompute summaries; the caller batches that.
    pub(super) fn write_change(&self, change: &Change) -> Result<Option<ThreadId>, StoreError> {
        let thread = match change {
            Change::MessageRead(id, state) => {
                self.connection().execute(
                    "UPDATE messages SET read = ?2 WHERE id = ?1",
                    params![id.to_string(), to_json("ReadState", state)?],
                )?;
                self.thread_of(*id)?
            }
            Change::MessageStar(id, star) => {
                self.connection().execute(
                    "UPDATE messages SET star = ?2 WHERE id = ?1",
                    params![id.to_string(), to_json("Star", star)?],
                )?;
                self.thread_of(*id)?
            }
            Change::MessageMailbox(id, role) => {
                self.connection().execute(
                    "UPDATE messages SET mailbox = ?2 WHERE id = ?1",
                    params![id.to_string(), to_json("MailboxRole", role)?],
                )?;
                self.thread_of(*id)?
            }
            Change::MessageLabel(id, label, membership) => {
                match membership {
                    Membership::In => {
                        self.connection().execute(
                            "INSERT OR IGNORE INTO message_labels (message, label) VALUES (?1, ?2)",
                            params![id.to_string(), label.to_string()],
                        )?;
                    }
                    Membership::Out => {
                        self.connection().execute(
                            "DELETE FROM message_labels WHERE message = ?1 AND label = ?2",
                            params![id.to_string(), label.to_string()],
                        )?;
                    }
                }
                self.thread_of(*id)?
            }
            Change::ThreadSnooze(id, snooze) => {
                self.connection().execute(
                    "UPDATE threads SET snooze = ?2 WHERE id = ?1",
                    params![id.to_string(), to_json("Snooze", snooze)?],
                )?;
                Some(*id)
            }
            Change::ThreadPin(id, pin) => {
                self.connection().execute(
                    "UPDATE threads SET pin = ?2 WHERE id = ?1",
                    params![id.to_string(), to_json("Pin", pin)?],
                )?;
                Some(*id)
            }
            Change::MessageUpsert(msg) => {
                self.upsert_message(msg)?;
                Some(msg.thread)
            }
            Change::MessageDelete(id) => {
                let thread = self.thread_of(*id)?;
                self.connection().execute(
                    "DELETE FROM messages WHERE id = ?1",
                    params![id.to_string()],
                )?;
                thread
            }
            Change::LabelUpsert(label) => {
                self.connection().execute(
                    "INSERT INTO labels (id, account, name, color, origin) VALUES (?1,?2,?3,?4,?5)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name, color=excluded.color,
                                                   origin=excluded.origin",
                    params![
                        label.id.to_string(),
                        label.account.to_string(),
                        label.name,
                        label.color,
                        to_json("LabelOrigin", &label.origin)?,
                    ],
                )?;
                None
            }
            // A draft belongs to no thread, so there is no summary to refresh: `None`
            // here means "nothing to recompute", not "nothing was written".
            Change::DraftUpsert(draft) => {
                self.write_draft(draft)?;
                None
            }
            Change::DraftDelete(id) => {
                self.delete_draft(*id)?;
                None
            }
        };
        Ok(thread)
    }

    fn thread_of(&self, message: MessageId) -> Result<Option<ThreadId>, StoreError> {
        let found: Option<String> = self
            .connection()
            .query_row(
                "SELECT thread FROM messages WHERE id = ?1",
                params![message.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        found
            .map(|t| uuid("ThreadId", &t).map(ThreadId::from_uuid))
            .transpose()
    }

    pub(super) fn upsert_message(&self, m: &Message) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT OR IGNORE INTO threads (id, account, snooze, pin)
             VALUES (?1, ?2, '{\"kind\":\"inactive\"}', '{\"kind\":\"unpinned\"}')",
            params![m.thread.to_string(), m.account.to_string()],
        )?;
        let recipients = Recipients {
            reply_to: m.reply_to.clone(),
            to: m.to.clone(),
            cc: m.cc.clone(),
            bcc: m.bcc.clone(),
        };
        self.connection().execute(
            "INSERT INTO messages (id, thread, account, msg_key, date, from_name, from_email,
                 recipients, subject, in_reply_to, refs, rfc_message_id, read, star, mailbox,
                 body_text, body_raw, attachments, fts_text)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
             ON CONFLICT(id) DO UPDATE SET
                 thread=excluded.thread, subject=excluded.subject, read=excluded.read,
                 star=excluded.star, mailbox=excluded.mailbox, body_text=excluded.body_text,
                 attachments=excluded.attachments, recipients=excluded.recipients,
                 fts_text=excluded.fts_text",
            params![
                m.id.to_string(),
                m.thread.to_string(),
                m.account.to_string(),
                to_json("MessageKey", &m.key)?,
                from_time(m.date),
                m.from.name,
                m.from.email,
                to_json("Recipients", &recipients)?,
                m.subject,
                m.in_reply_to,
                to_json("references", &m.references)?,
                m.rfc_message_id,
                to_json("ReadState", &m.read)?,
                to_json("Star", &m.star)?,
                to_json("MailboxRole", &m.mailbox)?,
                m.body.text(),
                m.body.raw().map(|b| b.to_string()),
                to_json("attachments", &m.attachments)?,
                // What the index will hold: the tokens the query side will ask for, and nothing
                // else. See `crate::sql::indexable`.
                crate::sql::indexable(&[
                    Some(m.subject.as_str()),
                    m.from.name.as_deref(),
                    Some(m.from.email.as_str()),
                    m.body.text(),
                ]),
            ],
        )?;
        for label in &m.labels {
            self.connection().execute(
                "INSERT OR IGNORE INTO message_labels (message, label) VALUES (?1, ?2)",
                params![m.id.to_string(), label.to_string()],
            )?;
        }
        Ok(())
    }

    /// Rebuild the materialized summary for `thread`, or drop it if the thread is now empty.
    pub(super) fn refresh_summary(&self, thread: ThreadId) -> Result<(), StoreError> {
        // The writer, not a reader: this runs inside `write_patch`'s transaction and has to
        // see the rows it has just written.
        let messages = self.messages_of(&self.connection(), thread)?;
        if messages.is_empty() {
            self.connection().execute(
                "DELETE FROM threads WHERE id = ?1",
                params![thread.to_string()],
            )?;
            return Ok(());
        }
        let (snooze, pin): (String, String) = self.connection().query_row(
            "SELECT snooze, pin FROM threads WHERE id = ?1",
            params![thread.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let s = ThreadSummary::derive(
            thread,
            &messages,
            json("Snooze", &snooze)?,
            json("Pin", &pin)?,
        );
        self.connection().execute(
            "INSERT INTO thread_summary (thread, account, subject, snippet, from_name, from_email,
                 participants, recipients, last_date, message_count, read, star, mailboxes,
                 labels, attachments, snooze, pin)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(thread) DO UPDATE SET
                 subject=excluded.subject, snippet=excluded.snippet, from_name=excluded.from_name,
                 from_email=excluded.from_email, participants=excluded.participants,
                 recipients=excluded.recipients, last_date=excluded.last_date,
                 message_count=excluded.message_count, read=excluded.read, star=excluded.star,
                 mailboxes=excluded.mailboxes, labels=excluded.labels,
                 attachments=excluded.attachments, snooze=excluded.snooze, pin=excluded.pin",
            params![
                s.id.to_string(),
                s.account.to_string(),
                s.subject,
                s.snippet,
                s.from.name,
                s.from.email,
                to_json("participants", &s.participants)?,
                to_json("recipients", &s.recipients)?,
                from_time(s.last_date),
                i64::from(s.message_count),
                to_json("ReadState", &s.read)?,
                to_json("Star", &s.star)?,
                to_json("MailboxSet", &s.mailboxes)?,
                to_json("labels", &s.labels)?,
                to_json("Attachments", &s.attachments)?,
                to_json("Snooze", &s.snooze)?,
                to_json("Pin", &s.pin)?,
            ],
        )?;
        Ok(())
    }

    /// Write a local patch, atomically, and rebuild whatever summaries it disturbed.
    pub(super) fn write_patch(&self, patch: &Patch) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let mut touched = BTreeSet::new();
        for change in &patch.changes {
            if let Some(t) = self.write_change(change)? {
                touched.insert(t);
            }
        }
        for thread in touched {
            self.refresh_summary(thread)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Write server truth, then put still-pending local changes back on top.
    ///
    /// Step 3 is the one that matters and the one an obvious implementation omits. Without it,
    /// the poll after a user stars a message sees the server's unstarred value, writes it, and
    /// the star flips back under the cursor. The local change is not wrong — it simply has not
    /// reached the server yet — so server truth is the *base*, not the last word.
    pub(super) fn write_ingest(
        &self,
        account: AccountId,
        ingest: Ingest,
    ) -> Result<Patch, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let mut changes: Vec<Change> = Vec::new();
        let mut touched: BTreeSet<ThreadId> = BTreeSet::new();

        // 1. A UIDVALIDITY reset invalidates every uid for this mailbox at once. The messages
        //    stay — they are still real — but nothing may be addressed by the old uids again.
        if ingest.validity == UidValidity::Reset {
            self.connection().execute(
                "DELETE FROM remote_map WHERE account = ?1 AND mailbox = ?2",
                params![account.to_string(), ingest.mailbox.path],
            )?;
        }

        for label in &ingest.labels {
            self.write_change(&Change::LabelUpsert(label.clone()))?;
            changes.push(Change::LabelUpsert(label.clone()));
        }

        // 2. Messages. Identity is MessageKey, never RemoteRef: the same message appears in
        //    INBOX and All Mail under different uids, so keying on the ref would store it twice.
        for fetched in &ingest.messages {
            let existing = self.message_by_key(account, &fetched.key)?;
            let id = match existing {
                Some(id) => {
                    // A message we already hold, arriving again. That is normal and usually a
                    // re-map — but it is ALSO how a body arrives for something fetched
                    // headers-first, and ignoring it outright meant the body pass could never
                    // fill anything in. Found by an end-to-end test, not by review.
                    if let Body::Present { .. } = fetched.message.body {
                        let held = self.body_of(id)?;
                        if matches!(held, Some(Body::Absent) | None) {
                            let mut filled = fetched.message.clone();
                            filled.id = id;
                            // Keep the flags we already hold: the body arriving says nothing
                            // about whether the user has read it.
                            self.fill_body(id, &filled)?;
                            changes.push(Change::MessageUpsert(Box::new(filled)));
                            if let Some(t) = self.thread_of(id)? {
                                touched.insert(t);
                            }
                        }
                    }
                    id
                }
                None => {
                    self.upsert_message(&fetched.message)?;
                    changes.push(Change::MessageUpsert(Box::new(fetched.message.clone())));
                    touched.insert(fetched.message.thread);
                    fetched.message.id
                }
            };
            self.map_remote(account, &fetched.remote, id)?;
            if let Some(t) = self.thread_of(id)? {
                touched.insert(t);
            }
        }

        // 3. Flag-only updates, far cheaper than refetching a message.
        for (remote, read, star) in &ingest.flags {
            if let Some(id) = self.message_by_remote(account, remote)? {
                self.write_change(&Change::MessageRead(id, *read))?;
                self.write_change(&Change::MessageStar(id, *star))?;
                changes.push(Change::MessageRead(id, *read));
                changes.push(Change::MessageStar(id, *star));
                if let Some(t) = self.thread_of(id)? {
                    touched.insert(t);
                }
            }
        }

        // 3b. Labels the server reports, where it has them — Gmail and nowhere else.
        //
        //     The list is complete rather than additive, like flags: a label the server no
        //     longer lists has been removed there, and a client that only ever adds accumulates
        //     labels the user deleted years ago. So the difference is applied in both
        //     directions, and only the difference — writing every membership every pass would
        //     fill the change log with nothing.
        for (remote, names) in &ingest.label_names {
            let Some(id) = self.message_by_remote(account, remote)? else {
                continue;
            };
            let mut wanted = Vec::new();
            for name in names {
                wanted.push(self.label_by_name(account, name)?);
            }
            let held = self.labels_of(&self.connection(), id)?;
            for label in wanted.iter().filter(|l| !held.contains(l)) {
                let change = Change::MessageLabel(id, *label, Membership::In);
                self.write_change(&change)?;
                changes.push(change);
            }
            for label in held.iter().filter(|l| !wanted.contains(l)) {
                let change = Change::MessageLabel(id, *label, Membership::Out);
                self.write_change(&change)?;
                changes.push(change);
            }
            if let Some(t) = self.thread_of(id)? {
                touched.insert(t);
            }
        }

        // 4. Expunged elsewhere. Drop the mapping; drop the message only when no mailbox still
        //    holds it, because vanishing from INBOX is what archiving looks like on Gmail.
        for remote in &ingest.gone {
            if let Some(id) = self.message_by_remote(account, remote)? {
                let (acct, mailbox, uidvalidity, uid, uidl) = remote_key(account, remote);
                self.connection().execute(
                    "DELETE FROM remote_map WHERE account=?1 AND mailbox=?2
                     AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
                    params![acct, mailbox, uidvalidity, uid, uidl],
                )?;
                let remaining: i64 = self.connection().query_row(
                    "SELECT count(*) FROM remote_map WHERE message = ?1",
                    params![id.to_string()],
                    |r| r.get(0),
                )?;
                if remaining == 0 {
                    if let Some(t) = self.thread_of(id)? {
                        touched.insert(t);
                    }
                    self.write_change(&Change::MessageDelete(id))?;
                    changes.push(Change::MessageDelete(id));
                }
            }
        }

        // 5. Re-layer. Server truth is the base; anything the user did that the server has not
        //    confirmed goes back on top of it.
        for thread in &touched {
            for pending in self.pending_for_thread(*thread)? {
                self.write_change(&pending)?;
                changes.push(pending);
            }
        }

        // 6. Cursor, so the next sync resumes rather than refetching.
        //
        // Only when this ingest actually knows where the mailbox got to. A header or body batch
        // carries `None`: it was handed a list of messages and fetched them, and never asked
        // the server what exists. Writing a cursor from one of those is how an IMAP account's
        // `UIDVALIDITY`, `UIDNEXT` and `HIGHESTMODSEQ` were destroyed milliseconds after the
        // survey recorded them, on every pass, for ever.
        if let Some(cursor) = &ingest.cursor {
            self.connection().execute(
                "INSERT INTO sync_state (account, mailbox, cursor, synced_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(account, mailbox) DO UPDATE SET
                     cursor=excluded.cursor, synced_at=excluded.synced_at",
                params![
                    account.to_string(),
                    ingest.mailbox.path,
                    to_json("SyncCursor", cursor)?,
                ],
            )?;
        }

        for thread in touched {
            self.refresh_summary(thread)?;
        }
        tx.commit()?;

        Ok(Patch {
            id: mail_domain::ChangeId::generate(),
            changes,
        })
    }

    /// What body, if any, is held for this message.
    fn body_of(&self, message: MessageId) -> Result<Option<Body>, StoreError> {
        let held: Option<Option<String>> = self
            .connection()
            .query_row(
                "SELECT body_raw FROM messages WHERE id = ?1",
                params![message.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(held.map(|raw| match raw {
            Some(_) => Body::Present {
                text: None,
                raw: mail_domain::BlobId::generate(),
            },
            None => Body::Absent,
        }))
    }

    /// Attach a body to a message we already hold, leaving its flags alone.
    ///
    /// Not `upsert_message`: that would also write `read` and `star` from the fetched copy,
    /// and a body arriving says nothing about whether the user has read it.
    fn fill_body(&self, message: MessageId, filled: &Message) -> Result<(), StoreError> {
        let Body::Present { text, raw } = &filled.body else {
            return Ok(());
        };
        // `fts_text` too, or the body never reaches the index: a message fetched headers-first
        // was indexed with a subject and a sender, and this is the moment its text arrives. An
        // end-to-end test caught that the day the index stopped reading `body_text` directly.
        self.connection().execute(
            "UPDATE messages SET body_text = ?2, body_raw = ?3, attachments = ?4,
                 fts_text = ?5 WHERE id = ?1",
            params![
                message.to_string(),
                text,
                raw.to_string(),
                to_json("attachments", &filled.attachments)?,
                crate::sql::indexable(&[
                    Some(filled.subject.as_str()),
                    filled.from.name.as_deref(),
                    Some(filled.from.email.as_str()),
                    text.as_deref(),
                ]),
            ],
        )?;
        Ok(())
    }

    fn message_by_key(
        &self,
        account: AccountId,
        key: &mail_domain::MessageKey,
    ) -> Result<Option<MessageId>, StoreError> {
        let found: Option<String> = self
            .connection()
            .query_row(
                "SELECT id FROM messages WHERE account = ?1 AND msg_key = ?2",
                params![account.to_string(), to_json("MessageKey", key)?],
                |r| r.get(0),
            )
            .optional()?;
        found
            .map(|t| uuid("MessageId", &t).map(MessageId::from_uuid))
            .transpose()
    }

    fn message_by_remote(
        &self,
        account: AccountId,
        remote: &RemoteRef,
    ) -> Result<Option<MessageId>, StoreError> {
        let (acct, mailbox, uidvalidity, uid, uidl) = remote_key(account, remote);
        let found: Option<String> = self
            .connection()
            .query_row(
                "SELECT message FROM remote_map WHERE account=?1 AND mailbox=?2
                 AND uidvalidity IS ?3 AND uid IS ?4 AND uidl IS ?5",
                params![acct, mailbox, uidvalidity, uid, uidl],
                |r| r.get(0),
            )
            .optional()?;
        found
            .map(|t| uuid("MessageId", &t).map(MessageId::from_uuid))
            .transpose()
    }

    /// The id of the label called `name` on this account, creating it if it is new.
    ///
    /// `LabelOrigin::Provider`: the server made it, so the user may not rename or delete it here
    /// — that is what the origin is for. The `UNIQUE (account, name)` constraint is what makes
    /// this safe against two passes racing to create the same one.
    fn label_by_name(
        &self,
        account: AccountId,
        name: &str,
    ) -> Result<mail_domain::LabelId, StoreError> {
        let existing: Option<String> = self
            .connection()
            .query_row(
                "SELECT id FROM labels WHERE account = ?1 AND name = ?2",
                params![account.to_string(), name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return uuid("LabelId", &id).map(mail_domain::LabelId::from_uuid);
        }
        let label = mail_domain::Label {
            id: mail_domain::LabelId::generate(),
            account,
            name: name.to_owned(),
            color: None,
            origin: mail_domain::LabelOrigin::Provider,
        };
        self.write_change(&Change::LabelUpsert(label.clone()))?;
        Ok(label.id)
    }

    fn map_remote(
        &self,
        account: AccountId,
        remote: &RemoteRef,
        message: MessageId,
    ) -> Result<(), StoreError> {
        let (acct, mailbox, uidvalidity, uid, uidl) = remote_key(account, remote);
        // Many-to-one on purpose: one message, several mailboxes, several uids.
        // `ON CONFLICT` naming the expression index, not `INSERT OR REPLACE` on the primary
        // key: every row has a NULL in that key (exactly one of uid/uidl is set), SQLite treats
        // NULLs as distinct there, and so nothing ever conflicted. See migration 0002.
        self.connection().execute(
            "INSERT INTO remote_map
                 (account, mailbox, uidvalidity, uid, uidl, message)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT (account, mailbox, COALESCE(uidvalidity, -1),
                          COALESCE(uid, -1), COALESCE(uidl, ''))
             DO UPDATE SET message = excluded.message",
            params![acct, mailbox, uidvalidity, uid, uidl, message.to_string()],
        )?;
        Ok(())
    }

    /// Local changes on this thread's messages that the server has not confirmed.
    fn pending_for_thread(&self, thread: ThreadId) -> Result<Vec<Change>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT p.changes FROM pending_changes p
             JOIN messages m ON m.id = p.message
             WHERE m.thread = ?1
             ORDER BY p.outbox",
        )?;
        let rows = stmt.query_map(params![thread.to_string()], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for text in rows {
            let batch: Vec<Change> = json("pending changes", &text?)?;
            out.extend(batch);
        }
        Ok(out)
    }
}
