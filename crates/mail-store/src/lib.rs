//! Local persistence: SQLite with FTS5, plus the blob store on disk.
//!
//! Does disk I/O. Opens no sockets. The schema is `migrations/0001_initial.sql`.

pub mod error;

pub use error::StoreError;

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, Filter, Ingest, Message, MessageId, OutboxId, Page, Patch, ProtoOp, Query,
    RemoteIntent, Retry, Thread, ThreadId, ThreadSummary,
};

/// One queued unit of remote work, with everything needed to retry or abandon it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub id: OutboxId,
    pub account: AccountId,
    pub op: ProtoOp,
    /// Applied if this entry ends in [`Retry::Fatal`], so an optimistic local change does not
    /// outlive an operation the server refused.
    pub undo: Patch,
    pub attempts: u32,
    pub next_attempt: DateTime<Utc>,
}

/// How a queued operation finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// Confirmed. Pending markers for its messages are cleared.
    Ok,
    /// Failed. The store reschedules, prompts for reauth, or applies the undo patch and drops
    /// the entry, according to the [`Retry`].
    Failed { reason: String, retry: Retry },
}

/// Query and persist. The only writer the UI ever calls is [`Store::apply`].
pub trait Store {
    /// One page of a view.
    ///
    /// Paginated because a view over a real mailbox does not fit in memory, and because this
    /// signature propagates into every caller — widening it later is a workspace-wide edit.
    /// `now` resolves [`Filter::SnoozeDue`].
    fn threads(&self, query: &Query, now: DateTime<Utc>)
    -> Result<Page<ThreadSummary>, StoreError>;

    /// How many threads match, without loading them. Sidebar badges need this; loading every
    /// row to count it is how a mail client becomes unusable.
    fn count(&self, filter: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError>;

    fn thread(&self, id: ThreadId) -> Result<Thread, StoreError>;

    fn message(&self, id: MessageId) -> Result<Message, StoreError>;

    /// Write local mutations, atomically.
    fn apply(&self, account: AccountId, patch: &Patch) -> Result<(), StoreError>;

    /// Write server truth, then re-layer any still-pending local changes on top of it.
    ///
    /// The re-layering is not optional. Without it, the poll after starring a message sees the
    /// server's unstarred value, writes it, and the star flips back under the user's cursor.
    ///
    /// Returns a [`Patch`] describing what actually moved, so the UI refreshes precisely
    /// instead of re-running every open query.
    fn ingest(&self, account: AccountId, ingest: Ingest) -> Result<Patch, StoreError>;

    /// Queue remote work, recording `undo` and marking the affected messages pending.
    ///
    /// Takes a [`RemoteIntent`] — addressed in local ids — and resolves it into a [`ProtoOp`]
    /// here, because this is the only place where `remote_map` (local id to [`mail_domain::RemoteRef`],
    /// many-to-one) and `labels` (id to server-side name) both exist. `Op::apply` cannot do it:
    /// see `RemoteIntent`'s own documentation.
    ///
    /// Returns `Ok(None)` when the intent resolves to nothing to say — every named message is
    /// unknown to the server, which is normal for a message composed locally and not yet sent.
    fn enqueue(
        &self,
        account: AccountId,
        intent: RemoteIntent,
        undo: &Patch,
        now: DateTime<Utc>,
    ) -> Result<Option<OutboxId>, StoreError>;

    /// Queued work whose `next_attempt` has arrived, in insertion order.
    ///
    /// Insertion order is load-bearing: two operations on one thread, with an [`Ingest`]
    /// landing between them, otherwise have no defined result.
    fn outbox_due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError>;

    /// Record how a queued operation finished and act on it.
    fn outbox_settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}
