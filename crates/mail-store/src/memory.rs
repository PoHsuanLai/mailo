//! An in-memory [`Store`], for tests and for the parity proptest.
//!
//! Its query path is implemented *by calling* [`mail_domain::Filter::fit`] — never by a second
//! hand-written matcher. A second matcher would be a third implementation of the same
//! semantics, and the parity test would then be comparing two wrongs.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use mail_domain::{
    AccountCaps, AccountId, Change, ChangeId, Cursor, Draft, DraftId, Filter, Ingest, Label,
    LabelId, MailboxRef, MatchCtx, Membership, Message, MessageId, MessageKey, OutboxId, Page,
    Patch, Pin, Property, ProtoOp, Query, RemoteIntent, RemoteRef, Retry, SendState, Snooze,
    SortDir, SyncCursor, Thread, ThreadId, ThreadSummary, UidValidity,
};
use serde::Serialize;

use crate::{OutboxEntry, Settle, Store, StoreError};

/// Everything held in memory. Cheap to construct, and never touches the disk.
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: RefCell<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Accounts that have appeared in a write. `enqueue` refuses a resolved operation for an
    /// account that was never written, the way the SQL foreign key would.
    accounts: BTreeSet<AccountId>,
    threads: BTreeMap<ThreadId, ThreadState>,
    messages: BTreeMap<MessageId, Message>,
    /// Message identity. Many [`RemoteRef`]s may point at one entry; the key is not the ref.
    by_key: HashMap<AccountId, HashMap<MessageKey, MessageId>>,
    remotes: Vec<RemoteRow>,
    labels: BTreeMap<LabelId, Label>,
    drafts: BTreeMap<DraftId, Draft>,
    /// What each account's server turned out to support.
    caps: BTreeMap<AccountId, AccountCaps>,
    outbox: BTreeMap<OutboxId, OutboxRow>,
    next_outbox: i64,
    pending: Vec<PendingRow>,
    sync: BTreeMap<(AccountId, String), SyncCursor>,
}

#[derive(Debug, Clone, Copy)]
struct ThreadState {
    snooze: Snooze,
    pin: Pin,
}

#[derive(Debug, Clone)]
struct RemoteRow {
    account: AccountId,
    mailbox: String,
    uidvalidity: Option<u32>,
    uid: Option<u32>,
    uidl: Option<String>,
    message: MessageId,
}

#[derive(Debug, Clone)]
struct OutboxRow {
    account: AccountId,
    op: ProtoOp,
    undo: Patch,
    attempts: u32,
    next_attempt: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct PendingRow {
    message: MessageId,
    outbox: OutboxId,
    changes: Vec<Change>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            accounts: BTreeSet::new(),
            threads: BTreeMap::new(),
            messages: BTreeMap::new(),
            by_key: HashMap::new(),
            remotes: Vec::new(),
            labels: BTreeMap::new(),
            drafts: BTreeMap::new(),
            caps: BTreeMap::new(),
            outbox: BTreeMap::new(),
            // SQLite rowids start at 1. Matching that keeps insertion order obvious in tests.
            next_outbox: 1,
            pending: Vec::new(),
            sync: BTreeMap::new(),
        }
    }
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemoryStore {
    fn threads(
        &self,
        query: &Query,
        now: DateTime<Utc>,
    ) -> Result<Page<ThreadSummary>, StoreError> {
        self.inner.borrow().threads(query, now)
    }

    fn count(&self, filter: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError> {
        Ok(self.inner.borrow().matching(filter, now).len() as u64)
    }

    fn thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        let inner = self.inner.borrow();
        let summary = inner.summary_of(id).ok_or(StoreError::NoThread(id))?;
        let messages = inner.messages_of(id).into_iter().map(|m| m.id).collect();
        Ok(Thread { summary, messages })
    }

    fn cursor(&self, mailbox: &MailboxRef) -> Result<Option<SyncCursor>, StoreError> {
        // `sync` has held this since the store was written; nothing had ever read it back.
        Ok(self
            .inner
            .borrow()
            .sync
            .get(&(mailbox.account, mailbox.path.clone()))
            .cloned())
    }

    fn put_caps(
        &self,
        account: AccountId,
        caps: &AccountCaps,
        _now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner.borrow_mut().caps.insert(account, caps.clone());
        Ok(())
    }

    fn remote_refs(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, StoreError> {
        let inner = self.inner.borrow();
        Ok(inner
            .remotes
            .iter()
            .filter(|row| row.account == mailbox.account && row.mailbox == mailbox.path)
            .map(|row| match (row.uid, &row.uidl) {
                (Some(uid), None) => RemoteRef::Imap {
                    mailbox: row.mailbox.clone(),
                    uidvalidity: row.uidvalidity.unwrap_or(0),
                    uid,
                },
                _ => RemoteRef::Pop {
                    uidl: row.uidl.clone().unwrap_or_default(),
                },
            })
            .collect())
    }

    fn remotes_of(&self, message: MessageId) -> Result<Vec<RemoteRef>, StoreError> {
        let inner = self.inner.borrow();
        if !inner.messages.contains_key(&message) {
            return Err(StoreError::NoMessage(message));
        }
        Ok(inner
            .remotes
            .iter()
            .filter(|row| row.message == message)
            .map(|row| match (row.uid, &row.uidl) {
                (Some(uid), None) => RemoteRef::Imap {
                    mailbox: row.mailbox.clone(),
                    uidvalidity: row.uidvalidity.unwrap_or(0),
                    uid,
                },
                _ => RemoteRef::Pop {
                    uidl: row.uidl.clone().unwrap_or_default(),
                },
            })
            .collect())
    }

    fn hold_part(
        &self,
        message: MessageId,
        section: &str,
        blob: mail_domain::BlobId,
        size: u64,
    ) -> Result<(), StoreError> {
        let mut inner = self.inner.borrow_mut();
        let stored = inner
            .messages
            .get_mut(&message)
            .ok_or(StoreError::NoMessage(message))?;
        crate::sqlite::held(&mut stored.attachments, message, section, blob, size)
    }

    fn draft(&self, id: DraftId) -> Result<Draft, StoreError> {
        self.inner
            .borrow()
            .drafts
            .get(&id)
            .cloned()
            .ok_or(StoreError::NoDraft(id))
    }

    fn drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError> {
        let inner = self.inner.borrow();
        let mut out: Vec<Draft> = inner
            .drafts
            .values()
            .filter(|d| d.account == account)
            .cloned()
            .collect();
        // Newest first, matching SQLite's `ORDER BY updated_at DESC`. The id breaks ties so
        // the order is total: two drafts saved in the same second must not swap between calls,
        // or the parity test fails intermittently and for the wrong reason.
        out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
        Ok(out)
    }

    fn set_send_state(
        &self,
        id: DraftId,
        state: &SendState,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut inner = self.inner.borrow_mut();
        let draft = inner.drafts.get_mut(&id).ok_or(StoreError::NoDraft(id))?;
        draft.state = state.clone();
        draft.updated = now;
        Ok(())
    }

    fn message(&self, id: MessageId) -> Result<Message, StoreError> {
        let inner = self.inner.borrow();
        let mut message = inner
            .messages
            .get(&id)
            .cloned()
            .ok_or(StoreError::NoMessage(id))?;
        // `sqlite` reads `message_labels` ordered by label id. Same order here.
        message.labels.sort();
        Ok(message)
    }

    fn apply(&self, account: AccountId, patch: &Patch) -> Result<(), StoreError> {
        let mut inner = self.inner.borrow_mut();
        inner.accounts.insert(account);
        for change in &patch.changes {
            inner.write_change(change)?;
        }
        Ok(())
    }

    fn ingest(&self, account: AccountId, ingest: Ingest) -> Result<Patch, StoreError> {
        self.inner.borrow_mut().ingest(account, &ingest)
    }

    fn enqueue(
        &self,
        account: AccountId,
        intent: RemoteIntent,
        undo: &Patch,
        now: DateTime<Utc>,
    ) -> Result<Option<OutboxId>, StoreError> {
        self.inner.borrow_mut().enqueue(account, &intent, undo, now)
    }

    fn outbox_due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError> {
        Ok(self.inner.borrow().outbox_due(account, now))
    }

    fn unfetched(
        &self,
        account: AccountId,
        limit: u32,
    ) -> Result<Vec<mail_domain::RemoteRef>, StoreError> {
        let inner = self.inner.borrow();
        let mut with_dates: Vec<(chrono::DateTime<chrono::Utc>, mail_domain::RemoteRef)> = inner
            .messages
            .values()
            .filter(|m| m.account == account && matches!(m.body, mail_domain::Body::Absent))
            .filter_map(|m| {
                inner
                    .remotes
                    .iter()
                    .find(|row| row.message == m.id)
                    .and_then(|row| {
                        let remote = match (row.uid, row.uidl.clone()) {
                            (Some(uid), None) => mail_domain::RemoteRef::Imap {
                                mailbox: row.mailbox.clone(),
                                uidvalidity: row.uidvalidity.unwrap_or(0),
                                uid,
                            },
                            (_, Some(uidl)) => mail_domain::RemoteRef::Pop { uidl },
                            // The row invariant forbids this; skipping beats inventing a ref.
                            (None, None) => return None,
                        };
                        Some((m.date, remote))
                    })
            })
            .collect();
        with_dates.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
        Ok(with_dates
            .into_iter()
            .map(|(_, remote)| remote)
            .take(limit as usize)
            .collect())
    }

    fn outbox_settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner.borrow_mut().outbox_settle(id, settle, now)
    }
}

impl Inner {
    /// One page, filtered only by [`Filter::fit`].
    fn threads(
        &self,
        query: &Query,
        now: DateTime<Utc>,
    ) -> Result<Page<ThreadSummary>, StoreError> {
        let property = query.sort.property;
        let mut keyed = Vec::new();
        for summary in self.matching(&query.filter, now) {
            keyed.push((sort_key(&summary, property)?, summary));
        }
        keyed.sort_by(|a, b| {
            let ord = a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id));
            match query.sort.dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            }
        });
        if let Some(cursor) = &query.page.after {
            let (value, thread) = decode_cursor(cursor)?;
            keyed.retain(|(key, summary)| {
                after_cursor(key, summary.id, &value, &thread, query.sort.dir)
            });
        }
        let limit = usize::try_from(query.page.limit).unwrap_or(usize::MAX);
        let more = keyed.len() > limit;
        keyed.truncate(limit);
        let next = if more {
            keyed
                .last()
                .map(|(key, summary)| encode_cursor(key, summary.id))
        } else {
            None
        };
        Ok(Page {
            items: keyed.into_iter().map(|(_, summary)| summary).collect(),
            next,
        })
    }

    /// Threads whose summary [`Filter::fit`] accepts.
    ///
    /// `body_text` is every message body, oldest first, joined with newlines. Co-occurrence
    /// is defined on the whole thread, and [`MatchCtx`] has one body slot. A phrase can
    /// therefore sit across that join; FTS5 will not, because a phrase cannot cross rows.
    fn matching(&self, filter: &Filter, now: DateTime<Utc>) -> Vec<ThreadSummary> {
        let ids: Vec<ThreadId> = self.threads.keys().copied().collect();
        let mut out = Vec::new();
        for id in ids {
            let Some((summary, body)) = self.view(id) else {
                continue;
            };
            let ctx = MatchCtx {
                summary: &summary,
                corpus: body.as_deref(),
                now,
            };
            if filter.fit(&ctx) {
                out.push(summary);
            }
        }
        out
    }

    fn view(&self, id: ThreadId) -> Option<(ThreadSummary, Option<String>)> {
        let (snooze, pin) = {
            let state = self.threads.get(&id)?;
            (state.snooze, state.pin)
        };
        let messages = self.messages_of(id);
        if messages.is_empty() {
            return None;
        }
        let body = thread_corpus(&messages);
        let summary = ThreadSummary::derive(id, &messages, snooze, pin);
        Some((summary, body))
    }

    fn summary_of(&self, id: ThreadId) -> Option<ThreadSummary> {
        self.view(id).map(|(summary, _)| summary)
    }

    /// Oldest first, ties broken by id, labels in id order. That is the order
    /// `ThreadSummary::derive` sees when SQLite reloads a thread.
    fn messages_of(&self, thread: ThreadId) -> Vec<Message> {
        let mut messages: Vec<Message> = self
            .messages
            .values()
            .filter(|m| m.thread == thread)
            .cloned()
            .collect();
        messages.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));
        for message in &mut messages {
            message.labels.sort();
        }
        messages
    }

    fn write_change(&mut self, change: &Change) -> Result<(), StoreError> {
        match change {
            Change::MessageRead(id, state) => {
                if let Some(message) = self.messages.get_mut(id) {
                    message.read = *state;
                }
            }
            Change::MessageStar(id, star) => {
                if let Some(message) = self.messages.get_mut(id) {
                    message.star = *star;
                }
            }
            Change::MessageMailbox(id, role) => {
                if let Some(message) = self.messages.get_mut(id) {
                    message.mailbox = *role;
                }
            }
            Change::MessageLabel(id, label, membership) => {
                if let Some(message) = self.messages.get_mut(id) {
                    match membership {
                        Membership::In => {
                            if !message.labels.contains(label) {
                                message.labels.push(*label);
                            }
                        }
                        Membership::Out => message.labels.retain(|existing| existing != label),
                    }
                }
            }
            Change::ThreadSnooze(id, snooze) => {
                if let Some(thread) = self.threads.get_mut(id) {
                    thread.snooze = *snooze;
                }
            }
            Change::ThreadPin(id, pin) => {
                if let Some(thread) = self.threads.get_mut(id) {
                    thread.pin = *pin;
                }
            }
            Change::MessageUpsert(message) => self.upsert_message(message)?,
            Change::MessageDelete(id) => {
                self.delete_message(*id);
            }
            Change::LabelUpsert(label) => {
                self.accounts.insert(label.account);
                self.labels.insert(label.id, label.clone());
            }
            Change::DraftUpsert(draft) => {
                self.accounts.insert(draft.account);
                self.drafts.insert(draft.id, (**draft).clone());
            }
            Change::DraftDelete(id) => {
                self.drafts.remove(id);
            }
        }
        Ok(())
    }

    fn upsert_message(&mut self, message: &Message) -> Result<(), StoreError> {
        if let Some(other) = self
            .by_key
            .get(&message.account)
            .and_then(|map| map.get(&message.key))
            && *other != message.id
        {
            return Err(StoreError::Db(format!(
                "message key already maps to {other}"
            )));
        }
        let previous = self.messages.remove(&message.id);
        if let Some(prev) = &previous
            && let Some(map) = self.by_key.get_mut(&prev.account)
        {
            map.remove(&prev.key);
        }
        self.threads.entry(message.thread).or_insert(ThreadState {
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
        });
        self.by_key
            .entry(message.account)
            .or_default()
            .insert(message.key.clone(), message.id);
        self.accounts.insert(message.account);
        let old_thread = previous.map(|prev| prev.thread);
        self.messages.insert(message.id, message.clone());
        if let Some(old) = old_thread
            && old != message.thread
            && !self.has_messages(old)
        {
            self.threads.remove(&old);
        }
        Ok(())
    }

    fn delete_message(&mut self, id: MessageId) -> Option<ThreadId> {
        let prev = self.messages.remove(&id)?;
        if let Some(map) = self.by_key.get_mut(&prev.account) {
            map.remove(&prev.key);
        }
        self.remotes.retain(|row| row.message != id);
        self.pending.retain(|row| row.message != id);
        if !self.has_messages(prev.thread) {
            self.threads.remove(&prev.thread);
        }
        Some(prev.thread)
    }

    fn has_messages(&self, thread: ThreadId) -> bool {
        self.messages.values().any(|m| m.thread == thread)
    }

    fn thread_of(&self, id: MessageId) -> Option<ThreadId> {
        self.messages.get(&id).map(|m| m.thread)
    }

    /// Server truth, then still-pending local changes on the threads that moved.
    fn ingest(&mut self, account: AccountId, ingest: &Ingest) -> Result<Patch, StoreError> {
        self.accounts.insert(account);
        let mut changes = Vec::new();
        let mut touched = BTreeSet::new();

        if ingest.validity == UidValidity::Reset {
            self.remotes
                .retain(|row| !(row.account == account && row.mailbox == ingest.mailbox.path));
        }

        for label in &ingest.labels {
            let change = Change::LabelUpsert(label.clone());
            self.write_change(&change)?;
            changes.push(change);
        }

        for fetched in &ingest.messages {
            let id = if let Some(id) = self.message_by_key(account, &fetched.key) {
                id
            } else {
                self.upsert_message(&fetched.message)?;
                changes.push(Change::MessageUpsert(Box::new(fetched.message.clone())));
                touched.insert(fetched.message.thread);
                fetched.message.id
            };
            self.map_remote(account, &fetched.remote, id);
            if let Some(thread) = self.thread_of(id) {
                touched.insert(thread);
            }
        }

        for (remote, read, star) in &ingest.flags {
            if let Some(id) = self.message_by_remote(account, remote) {
                let read_change = Change::MessageRead(id, *read);
                let star_change = Change::MessageStar(id, *star);
                self.write_change(&read_change)?;
                self.write_change(&star_change)?;
                changes.push(read_change);
                changes.push(star_change);
                if let Some(thread) = self.thread_of(id) {
                    touched.insert(thread);
                }
            }
        }

        for remote in &ingest.gone {
            if let Some(id) = self.message_by_remote(account, remote) {
                self.remove_remote(account, remote);
                if !self.remotes.iter().any(|row| row.message == id) {
                    if let Some(thread) = self.thread_of(id) {
                        touched.insert(thread);
                    }
                    let change = Change::MessageDelete(id);
                    self.write_change(&change)?;
                    changes.push(change);
                }
            }
        }

        // Pending rows are the local changes the server has not confirmed. They go back on
        // top of whatever the poll just wrote, or a star flips back on the next sync.
        let pending = self.pending_for_threads(&touched);
        for change in &pending {
            self.write_change(change)?;
            changes.push(change.clone());
        }

        // Only a survey moves the cursor. A header or body batch has `None` and must leave
        // whatever the last survey recorded alone — overwriting it is what destroyed every
        // IMAP account's UIDVALIDITY and HIGHESTMODSEQ on every pass.
        if let Some(cursor) = &ingest.cursor {
            let _previous = self
                .sync
                .insert((account, ingest.mailbox.path.clone()), cursor.clone());
        }

        Ok(Patch {
            id: ChangeId::generate(),
            changes,
        })
    }

    fn message_by_key(&self, account: AccountId, key: &MessageKey) -> Option<MessageId> {
        self.by_key
            .get(&account)
            .and_then(|map| map.get(key))
            .copied()
    }

    fn message_by_remote(&self, account: AccountId, remote: &RemoteRef) -> Option<MessageId> {
        self.remotes
            .iter()
            .find(|row| same_remote(row, account, remote))
            .map(|row| row.message)
    }

    fn map_remote(&mut self, account: AccountId, remote: &RemoteRef, message: MessageId) {
        if let Some(row) = self
            .remotes
            .iter_mut()
            .find(|row| same_remote(row, account, remote))
        {
            row.message = message;
            return;
        }
        let (mailbox, uidvalidity, uid, uidl) = remote_parts(remote);
        self.remotes.push(RemoteRow {
            account,
            mailbox,
            uidvalidity,
            uid,
            uidl,
            message,
        });
    }

    fn remove_remote(&mut self, account: AccountId, remote: &RemoteRef) {
        self.remotes
            .retain(|row| !same_remote(row, account, remote));
    }

    fn pending_for_threads(&self, threads: &BTreeSet<ThreadId>) -> Vec<Change> {
        let mut rows: Vec<&PendingRow> = self
            .pending
            .iter()
            .filter(|row| {
                self.messages
                    .get(&row.message)
                    .is_some_and(|message| threads.contains(&message.thread))
            })
            .collect();
        rows.sort_by_key(|row| (row.outbox, row.message));
        rows.into_iter()
            .flat_map(|row| row.changes.clone())
            .collect()
    }

    fn enqueue(
        &mut self,
        account: AccountId,
        intent: &RemoteIntent,
        undo: &Patch,
        now: DateTime<Utc>,
    ) -> Result<Option<OutboxId>, StoreError> {
        let Some(op) = self.resolve_intent(account, intent)? else {
            return Ok(None);
        };
        if !self.accounts.contains(&account) {
            return Err(StoreError::Db(format!("no such account: {account}")));
        }
        let id = OutboxId::from_i64(self.next_outbox);
        self.next_outbox += 1;
        self.outbox.insert(
            id,
            OutboxRow {
                account,
                op,
                undo: undo.clone(),
                attempts: 0,
                next_attempt: now,
            },
        );
        for (message, changes) in pending_of(intent) {
            if changes.is_empty() {
                continue;
            }
            self.pending.push(PendingRow {
                message,
                outbox: id,
                changes,
            });
        }
        Ok(Some(id))
    }

    fn resolve_intent(
        &self,
        account: AccountId,
        intent: &RemoteIntent,
    ) -> Result<Option<ProtoOp>, StoreError> {
        // Mirrors SqliteStore::resolve_intent: a submission addresses no existing message.
        if let RemoteIntent::Send {
            draft,
            raw,
            mail_from,
            rcpt_to,
        } = intent
        {
            return Ok(Some(ProtoOp::Submit {
                draft: *draft,
                raw: *raw,
                mail_from: mail_from.clone(),
                rcpt_to: rcpt_to.clone(),
            }));
        }
        let messages = match intent {
            RemoteIntent::SetFlags { messages, .. }
            | RemoteIntent::SetMailbox { messages, .. }
            | RemoteIntent::SetLabels { messages, .. } => messages,
            RemoteIntent::Send { .. } => unreachable!("handled above"),
        };
        let remotes = self.refs_for(account, messages)?;
        if remotes.is_empty() {
            return Ok(None);
        }
        Ok(Some(match intent {
            RemoteIntent::SetFlags { read, star, .. } => ProtoOp::SetFlags {
                remotes,
                read: *read,
                star: *star,
            },
            RemoteIntent::SetMailbox { role, .. } => ProtoOp::SetMailbox {
                remotes,
                role: *role,
            },
            RemoteIntent::SetLabels { add, remove, .. } => ProtoOp::SetLabels {
                remotes,
                add: self.label_names(add),
                remove: self.label_names(remove),
            },
            RemoteIntent::Send { .. } => unreachable!("handled above"),
        }))
    }

    fn refs_for(
        &self,
        account: AccountId,
        messages: &[MessageId],
    ) -> Result<Vec<RemoteRef>, StoreError> {
        let mut out = Vec::new();
        for id in messages {
            for row in &self.remotes {
                if row.account == account && row.message == *id {
                    out.push(row_to_remote(row)?);
                }
            }
        }
        Ok(out)
    }

    fn label_names(&self, labels: &[LabelId]) -> Vec<String> {
        labels
            .iter()
            .filter_map(|id| self.labels.get(id).map(|label| label.name.clone()))
            .collect()
    }

    fn outbox_due(&self, account: AccountId, now: DateTime<Utc>) -> Vec<OutboxEntry> {
        self.outbox
            .iter()
            .filter(|(_, row)| row.account == account && row.next_attempt <= now)
            .map(|(id, row)| OutboxEntry {
                id: *id,
                account: row.account,
                op: row.op.clone(),
                undo: row.undo.clone(),
                attempts: row.attempts,
                next_attempt: row.next_attempt,
            })
            .collect()
    }

    fn outbox_settle(
        &mut self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        if !self.outbox.contains_key(&id) {
            return Err(StoreError::Db(format!("no such outbox entry: {id}")));
        }
        match settle {
            Settle::Ok => self.drop_entry(id),
            Settle::Failed { reason: _, retry } => match &retry {
                Retry::Now | Retry::After(_) => {
                    let attempts = self.outbox[&id].attempts;
                    let floor = match &retry {
                        Retry::After(delay) => {
                            TimeDelta::from_std(*delay).unwrap_or(TimeDelta::zero())
                        }
                        _ => TimeDelta::zero(),
                    };
                    // `attempts` is the count before this failure, matching `sqlite::outbox`.
                    let wait = backoff(attempts).max(floor);
                    let when = now.checked_add_signed(wait).ok_or_else(|| {
                        StoreError::Db("next_attempt overflowed DateTime".to_owned())
                    })?;
                    let row = self
                        .outbox
                        .get_mut(&id)
                        .ok_or_else(|| StoreError::Db(format!("no such outbox entry: {id}")))?;
                    row.attempts = attempts.saturating_add(1);
                    row.next_attempt = when;
                }
                Retry::NeedsReauth => {
                    let when = now.checked_add_signed(reauth_delay()).ok_or_else(|| {
                        StoreError::Db("next_attempt overflowed DateTime".to_owned())
                    })?;
                    self.outbox
                        .get_mut(&id)
                        .ok_or_else(|| StoreError::Db(format!("no such outbox entry: {id}")))?
                        .next_attempt = when;
                }
                Retry::Fatal(_) => {
                    let undo = self.outbox[&id].undo.clone();
                    self.drop_entry(id);
                    for change in &undo.changes {
                        self.write_change(change)?;
                    }
                }
            },
        }
        Ok(())
    }

    fn drop_entry(&mut self, id: OutboxId) {
        self.pending.retain(|row| row.outbox != id);
        self.outbox.remove(&id);
    }
}

/// Plain text of every message that has one, oldest first.
///
/// Kept in step with `thread_body` in `tests/parity.rs`. A newline is only a separator:
/// it adds no token, and it does not stop two tokens from being adjacent.
/// The full searchable text of a thread, as `messages_fts` indexes it.
///
/// Every message's subject, sender, `To`, `Cc` and body — not only the newest, and not only the
/// body. `ThreadSummary` carries the OLDEST message's subject and the NEWEST message's
/// sender, so a word appearing only in a reply's subject is findable in SQL and invisible to
/// `Filter::fit` unless it arrives through here. The parity proptest found exactly that.
fn thread_corpus(messages: &[Message]) -> Option<String> {
    let mut out = String::new();
    for message in messages {
        // The same fields, in the same order, as `sql::message_index`.
        let mut pieces = vec![
            Some(message.subject.as_str()),
            message.from.name.as_deref(),
            Some(message.from.email.as_str()),
        ];
        for addr in message.to.iter().chain(&message.cc) {
            pieces.push(addr.name.as_deref());
            pieces.push(Some(addr.email.as_str()));
        }
        pieces.push(message.body.text());
        for piece in pieces.into_iter().flatten() {
            if piece.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(piece);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Fixed nine-digit nanoseconds, the same text `sqlite::row::from_time` stores.
fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn sort_key(summary: &ThreadSummary, property: Property) -> Result<String, StoreError> {
    Ok(match property {
        Property::Date | Property::Size => timestamp(summary.last_date),
        Property::Subject => summary.subject.clone(),
        Property::From | Property::Sender => summary.from.email.clone(),
        Property::Attachments => json_text("Attachments", &summary.attachments)?,
        Property::Pin => json_text("Pin", &summary.pin)?,
    })
}

fn json_text<T: Serialize>(what: &str, value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError::Decode {
        what: what.to_owned(),
        why: e.to_string(),
    })
}

fn encode_cursor(sort_value: &str, thread: ThreadId) -> Cursor {
    Cursor(format!("{sort_value}\u{1f}{thread}"))
}

fn decode_cursor(cursor: &Cursor) -> Result<(String, String), StoreError> {
    cursor
        .0
        .split_once('\u{1f}')
        .map(|(value, thread)| (value.to_owned(), thread.to_owned()))
        .ok_or(StoreError::BadCursor)
}

fn after_cursor(key: &str, id: ThreadId, cursor_key: &str, cursor_id: &str, dir: SortDir) -> bool {
    let id = id.to_string();
    let ord = (key, id.as_str()).cmp(&(cursor_key, cursor_id));
    match dir {
        SortDir::Asc => ord == Ordering::Greater,
        SortDir::Desc => ord == Ordering::Less,
    }
}

fn remote_parts(remote: &RemoteRef) -> (String, Option<u32>, Option<u32>, Option<String>) {
    match remote {
        RemoteRef::Imap {
            mailbox,
            uidvalidity,
            uid,
        } => (mailbox.clone(), Some(*uidvalidity), Some(*uid), None),
        RemoteRef::Pop { uidl } => ("INBOX".to_owned(), None, None, Some(uidl.clone())),
    }
}

fn same_remote(row: &RemoteRow, account: AccountId, remote: &RemoteRef) -> bool {
    if row.account != account {
        return false;
    }
    let (mailbox, uidvalidity, uid, uidl) = remote_parts(remote);
    row.mailbox == mailbox && row.uidvalidity == uidvalidity && row.uid == uid && row.uidl == uidl
}

fn row_to_remote(row: &RemoteRow) -> Result<RemoteRef, StoreError> {
    match (row.uid, row.uidl.clone()) {
        (Some(uid), None) => Ok(RemoteRef::Imap {
            mailbox: row.mailbox.clone(),
            uidvalidity: row.uidvalidity.unwrap_or(0),
            uid,
        }),
        (None, Some(uidl)) => Ok(RemoteRef::Pop { uidl }),
        _ => Err(StoreError::Decode {
            what: "remote_map row".to_owned(),
            why: "row has neither a uid nor a uidl".to_owned(),
        }),
    }
}

fn pending_of(intent: &RemoteIntent) -> Vec<(MessageId, Vec<Change>)> {
    match intent {
        RemoteIntent::SetFlags {
            messages,
            read,
            star,
        } => messages
            .iter()
            .map(|id| {
                let mut changes = Vec::new();
                if let Some(read) = read {
                    changes.push(Change::MessageRead(*id, *read));
                }
                if let Some(star) = star {
                    changes.push(Change::MessageStar(*id, *star));
                }
                (*id, changes)
            })
            .collect(),
        RemoteIntent::SetMailbox { messages, role } => messages
            .iter()
            .map(|id| (*id, vec![Change::MessageMailbox(*id, *role)]))
            .collect(),
        RemoteIntent::SetLabels {
            messages,
            add,
            remove,
        } => messages
            .iter()
            .map(|id| {
                let mut changes = Vec::new();
                for label in add {
                    changes.push(Change::MessageLabel(*id, *label, Membership::In));
                }
                for label in remove {
                    changes.push(Change::MessageLabel(*id, *label, Membership::Out));
                }
                (*id, changes)
            })
            .collect(),
        // A submission re-layers nothing: it has no existing message to be overwritten by the
        // next ingest. Matches SqliteStore.
        RemoteIntent::Send { .. } => Vec::new(),
    }
}

/// 1s, 2s, 4s … capped at an hour. `attempts` is the count already recorded.
fn backoff(attempts: u32) -> TimeDelta {
    let secs = 1i64 << attempts.min(12);
    TimeDelta::try_seconds(secs.min(3600)).unwrap_or(TimeDelta::zero())
}

fn reauth_delay() -> TimeDelta {
    TimeDelta::try_hours(24)
        .unwrap_or_else(|| TimeDelta::try_seconds(24 * 3600).unwrap_or(TimeDelta::zero()))
}
