//! Local persistence: SQLite with FTS5, plus the blob store on disk.
//!
//! Does disk I/O. Opens no sockets. The schema is `migrations/0001_initial.sql`.

pub mod blob;
pub mod error;
pub mod memory;
mod memory_search;
pub mod migrate;
mod prefix;
pub mod sql;
pub mod sqlite;
mod term;

pub use memory::MemoryStore;
pub use sql::{SqlFilter, SqlValue, compile};
pub use sqlite::SqliteStore;
pub use term::Term;

pub use error::StoreError;

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, BlobId, Draft, DraftId, Filter, Folder, FolderContents, Ingest,
    MailboxRef, Message, MessageId, OutboxId, Page, Patch, ProtoOp, Query, RemoteIntent, RemoteRef,
    Retry, SendState, SyncCursor, Thread, ThreadId, ThreadSummary,
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

    /// Index terms that begin with `prefix`, most frequent first, at most `limit`.
    ///
    /// Every account's terms, on purpose: expanding a word with a term only another account has
    /// costs nothing, because the caller's account filter still applies to the results.
    ///
    /// The prefix is folded with [`mail_domain::filter::search_tokens`] before it is matched,
    /// because that is what the index stored. An empty prefix, or one that folds to no single
    /// token, returns an empty list rather than the vocabulary. Equal frequencies break by the
    /// term text, ascending.
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<Term>, StoreError>;

    /// The threads [`Store::threads`] would return for `filter`, each with a relevance score,
    /// best first.
    ///
    /// The score is the best (lowest) FTS5 `bm25()` among the thread's matching messages,
    /// **negated**, so higher is better. It is 0.0 when the filter has no text term.
    ///
    /// A filter with several text terms is scored from the MATCH arguments the full-text
    /// compiler already builds for [`Filter::Text`]: one quoted term per word of a `Contains`,
    /// one phrase for an `Exact`, combined with `OR` into one MATCH. The score is the minimum
    /// `bm25` among the thread's messages that hit that MATCH. `AND` inside the MATCH would
    /// require every word on the same message, and a thread whose words are split across
    /// messages would have no score even though [`Store::threads`] returns it. `bm25` cannot
    /// be aggregated directly, so the minimum is taken of the value the MATCH already produced.
    ///
    /// Capped at `limit`, ordered by that score and then by `last_date` descending. The
    /// in-memory store has no index score: every value is 0.0 and the order is `last_date`
    /// descending.
    fn search_ranked(
        &self,
        filter: &Filter,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<(ThreadSummary, f64)>, StoreError>;

    /// The best `k` of the `window` most recent threads [`Store::threads`] returns for
    /// `filter`, by full-text relevance, best first — a "Top results" strip, as Gmail and Apple
    /// Mail show above a date-ordered list.
    ///
    /// Relevance is scored as in [`Store::search_ranked`] (best `bm25` among the thread's
    /// matching messages, negated, higher is better), but only inside the window: the cost is
    /// bounded by `window`, not by how many messages a common word hits. A strong match older
    /// than the window is not a top hit; the list below finds it by date. Empty when the filter
    /// has no text term, since there is nothing to rank by.
    ///
    /// The in-memory store has no index score: it returns the first `k` of its window, newest
    /// first, each scored 0.0.
    fn top_hits(
        &self,
        filter: &Filter,
        k: usize,
        window: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<(ThreadSummary, f64)>, StoreError>;

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

    /// Remote addresses of messages we hold headers for but no body.
    ///
    /// `Body::Absent` is a normal state, not an error: a POP3 first sync fetches headers with
    /// `TOP` before any `RETR`, and IMAP fetches envelopes before bodies. This is how the
    /// runtime finds the work still outstanding after a restart, which is what makes a large
    /// first sync resumable rather than something that starts over.
    ///
    /// Newest first by the message date, and the order is part of the contract: `limit` cuts
    /// the list here, so whatever the caller would rather fetch first has to survive the cut.
    /// This was oldest-first, which under a budget meant the body pass worked through the
    /// maildrop from 2004 onwards and the mail that arrived this morning waited for all of it.
    fn unfetched(
        &self,
        account: AccountId,
        limit: u32,
    ) -> Result<Vec<mail_domain::RemoteRef>, StoreError>;

    /// Where the last sync of this mailbox got to, if one has finished.
    ///
    /// Written by every [`Store::ingest`]; nothing read it back until CONDSTORE needed the
    /// `HIGHESTMODSEQ` it had been recording all along.
    fn cursor(&self, mailbox: &MailboxRef) -> Result<Option<SyncCursor>, StoreError>;

    /// What the server turned out to support, so the next run starts from fact not guess.
    ///
    /// Written once at account creation from the preset's *expectation* and never updated,
    /// until this existed. Capabilities are discovered, not configured — that is the whole
    /// reason `AccountCaps` is a separate type from `AccountPlan`.
    fn put_caps(
        &self,
        account: AccountId,
        caps: &AccountCaps,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Every remote address this account holds in one mailbox.
    ///
    /// The other half of expunge detection: the server says what still exists, and this says
    /// what we think exists. What is in the second and not the first is gone.
    fn remote_refs(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, StoreError>;

    /// Every server address `message` is known by, in any mailbox.
    ///
    /// What fetching part of a message needs: the message is ours, the part is on the server,
    /// and any of its addresses can fetch it.
    fn remotes_of(&self, message: MessageId) -> Result<Vec<RemoteRef>, StoreError>;

    /// The attachment of `message` left on the server as `section` has arrived: it is `blob`,
    /// `size` bytes decoded.
    ///
    /// The bytes are already in the blob store; this records where. A section the message has
    /// no remote part for is [`StoreError::NoPart`], not a silent success, because the caller
    /// fetched something in order to put it there.
    fn hold_part(
        &self,
        message: MessageId,
        section: &str,
        blob: BlobId,
        size: u64,
    ) -> Result<(), StoreError>;

    /// One draft by id.
    fn draft(&self, id: DraftId) -> Result<Draft, StoreError>;

    /// Every draft on an account, most recently touched first.
    fn drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError>;

    /// Move a draft between send states.
    ///
    /// Separate from [`Store::apply`] because it is not the user's edit and must not be
    /// undoable: "Sending…" becoming "Sent" is the world reporting what happened, and an undo
    /// stack that could revert it would be lying about the message still being unsent.
    fn set_send_state(
        &self,
        id: DraftId,
        state: &SendState,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// Every mailbox this account's server lists, by path, with folder work still in the
    /// outbox laid on top.
    ///
    /// Empty until the first listing, and always for POP3, which has no folders to list.
    fn folders(&self, account: AccountId) -> Result<Vec<Folder>, StoreError>;

    /// Replace the account's folders with what the server just listed.
    ///
    /// Folder work still queued is re-applied over the listing, in queue order, for the reason
    /// [`Store::ingest`] re-layers pending changes: a listing taken before the server heard
    /// about a new folder would otherwise make that folder disappear until the outbox drained.
    fn put_folders(&self, account: AccountId, listed: Vec<Folder>) -> Result<(), StoreError>;

    /// What this client holds in one mailbox: the messages it has a server address for there,
    /// and the messages carrying the server's label of the same name.
    ///
    /// The question "would deleting this folder lose mail", asked of local knowledge. The
    /// server is asked again when the delete is sent, because a folder never synced holds
    /// nothing here whatever it holds there.
    fn folder_contents(&self, mailbox: &MailboxRef) -> Result<FolderContents, StoreError>;

    /// Record how a queued operation finished and act on it.
    ///
    /// A confirmed [`mail_domain::FolderWork::Delete`] also drops the deleted mailbox's server
    /// addresses and cursor, and any message that had no address anywhere else.
    fn outbox_settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}
