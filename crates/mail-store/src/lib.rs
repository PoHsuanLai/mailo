//! Local persistence: SQLite with FTS5, plus the blob store on disk.
//!
//! Does disk I/O. Opens no sockets. The schema is `migrations/0001_initial.sql`.

pub mod blob;
pub mod contact;
mod dispatch;
pub mod error;
mod filing;
pub mod memory;
mod memory_search;
pub mod migrate;
mod pgp;
mod prefix;
mod remote_row;
pub mod rules;
mod smime;
pub mod sql;
pub mod sqlite;
mod term;

pub use contact::{AddressBook, BookCard, Contact, Kind, Origin, Tally};
pub use dispatch::{PASSES_TO_FIND, SYNCS_TO_FIND};
pub use memory::MemoryStore;
pub use sql::{SqlFilter, SqlValue, compile};
pub use sqlite::SqliteStore;
pub use term::Term;

pub use error::StoreError;

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, AutocryptPeer, BlobId, Draft, DraftId, Filter, Fingerprint, Folder,
    FolderContents, Import, Ingest, InviteAnswer, KeyId, KeyTrust, Label, MailboxRef, MailboxRole,
    Message, MessageId, MessageKey, OutboxId, Page, Patch, PgpKey, ProtoOp, Query, ReceiptAnswer,
    RemoteIntent, RemoteRef, Retry, Rule, RuleId, SendState, SmimeCert, SyncCursor, Template,
    TemplateId, Thread, ThreadId, ThreadSummary, Vacation,
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

/// What a queued operation asks of the server at the moment it is sent.
///
/// Its messages' server addresses are looked up then, not when it was queued: an earlier
/// operation may have moved a message since, and the address it was queued with names where the
/// message was (FINDINGS F153).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// Send this: the queued operation, pointed at where its messages are now.
    Send(ProtoOp),
    /// Not yet. A message it names is held here at no server address — moved by a server that
    /// did not say where to — or an earlier queued operation on one of its messages is waiting.
    /// Left queued as it is, its attempts and its next attempt untouched, so the pass after the
    /// sync that finds the message sends it; it is not a failure and costs no backoff. Also the
    /// answer for an entry no longer queued, for which there is equally nothing to send.
    Wait,
    /// Nothing to send: every message it named has gone from this client. Settle it
    /// [`Settle::Ok`], which drops it.
    Moot,
    /// Given up: a message it names has waited as [`Dispatch::Wait`] through the syncs that
    /// should have found it ([`SYNCS_TO_FIND`], [`PASSES_TO_FIND`], FINDINGS F155). Settle it
    /// [`Settle::Failed`] with [`Retry::Fatal`] and this reason, in words for the user: its undo
    /// puts back what the server has, and the rest of the queue is no longer held behind it.
    Lost(String),
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

    /// The best `k` of `window` by full-text relevance to `filter`'s text terms, best first — a
    /// "Top results" strip, as Gmail and Apple Mail show above a date-ordered list.
    ///
    /// `window` is threads the caller has already listed with [`Store::threads`] for the same
    /// filter, typically its first page: listing them again here would evaluate the filter's
    /// full-text membership twice. Relevance is the best `bm25` among the thread's messages that
    /// hit any of the filter's text terms (one MATCH, the terms joined with `OR`), negated so
    /// higher is better. Only the window's messages are scored, so the cost is bounded by the
    /// window and not by how many messages a common word hits. A thread in the window with no
    /// message hitting a text term is not a top hit; ties go to the newer thread. Empty when the
    /// filter has no text term, since there is nothing to rank by.
    ///
    /// The in-memory store has no index score: it returns the first `k` of the window that
    /// match the filter's text terms, newest first, each scored 0.0.
    fn top_hits(
        &self,
        filter: &Filter,
        k: usize,
        window: &[ThreadId],
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

    /// Server truth about where messages are filed, from a protocol that says so outright.
    ///
    /// JMAP lists the mailboxes each changed email is in, and that decides its role here
    /// whether or not anything arrived: an email moved to Archive by another client changes
    /// nothing but its mailboxes. An [`Ingest`] can only file a message as it *arrives*, which
    /// needs its bytes, so this is the other half. Addresses the store does not hold are
    /// skipped, and a message already filed so is left alone.
    ///
    /// Re-layers pending local changes afterwards, exactly as `ingest` does: a message archived
    /// here and not yet moved there stays archived.
    fn refile(
        &self,
        account: AccountId,
        filed: &[(RemoteRef, MailboxRole)],
    ) -> Result<Patch, StoreError>;

    /// Keep messages that no server holds: imported mail, or an upload whose server did not say
    /// where it put it.
    ///
    /// Deduplicated by [`MessageKey`] exactly as [`Store::ingest`] is, so importing the same
    /// file twice holds each message once. A message already held gains any label it arrives
    /// with and loses none; its flags and body are left alone. Writes no `remote_map` row, so
    /// nothing done to these messages is ever queued for a server. Labels are created where
    /// new, as the user's own ([`mail_domain::LabelOrigin::User`]): no server owns them.
    ///
    /// Returns what changed, like `ingest`: a new message is a
    /// [`mail_domain::Change::MessageUpsert`] in it, and one already held is not.
    fn import(&self, account: AccountId, import: Import) -> Result<Patch, StoreError>;

    /// Whether this account already holds a message with this identity.
    ///
    /// What an upload asks before it queues anything, so re-importing into a server's mailbox
    /// does not upload the same message twice.
    fn holds(&self, account: AccountId, key: &MessageKey) -> Result<bool, StoreError>;

    /// Queue remote work, recording `undo` and marking the affected messages pending.
    ///
    /// Takes a [`RemoteIntent`] — addressed in local ids — and resolves it into a [`ProtoOp`]
    /// here, because this is the only place where `remote_map` (local id to [`mail_domain::RemoteRef`],
    /// many-to-one) and `labels` (id to server-side name) both exist. `Op::apply` cannot do it:
    /// see `RemoteIntent`'s own documentation.
    ///
    /// The addresses resolved here are only what the entry was queued with. The entry also keeps
    /// the messages that had one, and is pointed at where they are when it is sent
    /// ([`Store::outbox_dispatch`]); a message with no address now is left out of it for good.
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
    /// Each operation on messages is pointed at where its messages are now, as
    /// [`Store::outbox_dispatch`] would send it; one that would wait is listed as it was queued.
    /// A drain asks [`Store::outbox_dispatch`] again for each entry as it reaches it, because an
    /// entry sent before it may have moved one of its messages.
    ///
    /// Insertion order is load-bearing: two operations on one thread, with an [`Ingest`]
    /// landing between them, otherwise have no defined result.
    fn outbox_due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError>;

    /// What queued entry `id` asks of the server now: see [`Dispatch`].
    ///
    /// Asked by a drain for each entry immediately before sending it. Its messages' addresses
    /// are read from `remote_map` at that moment, so a move sent earlier in the same drain,
    /// remapped by [`Store::remap`] or let go of by [`Store::unmap`], is already accounted for.
    fn outbox_dispatch(&self, id: OutboxId) -> Result<Dispatch, StoreError>;

    /// The next moment, strictly after `after`, that queued work nobody has tried yet becomes
    /// due — in practice a send the user asked to go later, or one inside the window's grace
    /// period for Undo.
    ///
    /// What a watch sleeps until, so a scheduled send leaves on time rather than at the next
    /// time the server happens to say something. Strictly after, because an entry already due
    /// and still here after a pass is waiting on something a wake-up cannot fix, and counting
    /// it would wake the watch in a loop. Never-tried only, because a retry already has its own
    /// backoff and a pass is not worth waking for a one-second one.
    fn outbox_next(
        &self,
        account: AccountId,
        after: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, StoreError>;

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

    /// The same, for the messages that have an address in one mailbox: that address.
    ///
    /// What a body pass over one folder fetches. A fetch selects one mailbox and names UIDs in
    /// it, so a batch drawn from the whole account — as [`Store::unfetched`] returns it — asked
    /// the inbox for UIDs that belonged to Sent, and the answers were paired with the wrong
    /// addresses. Newest first, like `unfetched`, ties broken by message id.
    fn unfetched_in(
        &self,
        mailbox: &MailboxRef,
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

    /// The server moved a message and gave it a new address: `from` is now `to`.
    ///
    /// For a protocol whose addresses change with the folder — Microsoft Graph's ids do — so the
    /// next operation on the message names where it is rather than where it was. The message,
    /// its flags and its role stay as they are. Nothing happens if `from` is not held, and if a
    /// sync already mapped `to`, the row for `from` simply goes.
    fn remap(&self, account: AccountId, from: &RemoteRef, to: &RemoteRef)
    -> Result<(), StoreError>;

    /// The server moved a message and did not say where: `remote` no longer addresses it.
    ///
    /// Only the address goes; the message, its flags and its role stay as they are, and the
    /// sync that finds it in its new mailbox maps it again by its identity. Meanwhile a queued
    /// operation on it waits ([`Dispatch::Wait`]) rather than be sent to an address that names
    /// nothing, or something else. Nothing happens if `remote` is not held.
    ///
    /// `into` is the folder the move filed it in, where known: the syncs of that folder are the
    /// ones that should find it, and only they count against the wait ([`Store::unplaced_pass`]).
    /// Moved again before it was found, it starts waiting afresh.
    fn unmap(
        &self,
        account: AccountId,
        remote: &RemoteRef,
        into: Option<&str>,
    ) -> Result<(), StoreError>;

    /// One pass of `account` has synced the folders `synced` in full. Every message of the
    /// account still waiting to be found ([`Store::unmap`]) was not found by it, and this counts
    /// that against its wait: as a sync that missed it where its folder is among `synced`, and
    /// as a pass that did not look otherwise. Counted in passes rather than time, so a queue does
    /// not expire while the computer is shut; see [`Dispatch::Lost`].
    fn unplaced_pass(&self, account: AccountId, synced: &[String]) -> Result<(), StoreError>;

    /// Every server address `message` is known by, in any mailbox.
    ///
    /// What fetching part of a message needs: the message is ours, the part is on the server,
    /// and any of its addresses can fetch it.
    fn remotes_of(&self, message: MessageId) -> Result<Vec<RemoteRef>, StoreError>;

    /// Every mailbox `message` has a server address in, each with whether the message is still
    /// filed there here: what [`mail_domain::Filter::InFolder`] weighs, for a caller asking about
    /// one message rather than running a filter — a rule, an export.
    ///
    /// The role it is filed as, against [`mail_domain::FolderRoles::filed_as`] of each path by
    /// the account's last reported roles, and whether a move of it into another folder is still
    /// queued. [`StoreError::NoMessage`] for a message not held.
    fn placed(&self, message: MessageId) -> Result<Vec<mail_domain::Placed>, StoreError>;

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

    /// One template by id.
    fn template(&self, id: TemplateId) -> Result<Template, StoreError>;

    /// Every template on an account, by name without regard to ASCII case, then by id.
    ///
    /// By name rather than by recency, unlike drafts: a template is looked up, not resumed, and
    /// a list that reorders itself every time one is used is one nobody can learn.
    fn templates(&self, account: AccountId) -> Result<Vec<Template>, StoreError>;

    /// Keep a template, replacing any with the same id.
    ///
    /// Not through [`Store::apply`]: a template is not mail and not an undoable edit of any, and
    /// it never has a server side to reconcile with.
    fn put_template(&self, template: &Template) -> Result<(), StoreError>;

    /// Delete a template. [`StoreError::NoTemplate`] when there is none by that id, so a typo
    /// in an id is not reported as done.
    fn delete_template(&self, id: TemplateId) -> Result<(), StoreError>;

    /// Every label on an account, by name.
    ///
    /// On the trait because a rule names its labels and folders by name, and acting on one means
    /// finding the label that bears it — on either store.
    fn labels(&self, account: AccountId) -> Result<Vec<Label>, StoreError>;

    /// An account's rules, in the order they run: by position, then by name.
    fn rules(&self, account: AccountId) -> Result<Vec<Rule>, StoreError>;

    /// Keep a rule, replacing any with the same id.
    ///
    /// [`StoreError::RuleNameTaken`] when another rule on the account already has its name,
    /// because the name is how the command line finds a rule.
    fn put_rule(&self, rule: &Rule) -> Result<(), StoreError>;

    /// Delete a rule. [`StoreError::NoRule`] when there is none by that id.
    fn delete_rule(&self, id: RuleId) -> Result<(), StoreError>;

    /// The account's vacation reply, if one is set.
    fn vacation(&self, account: AccountId) -> Result<Option<Vacation>, StoreError>;

    /// Set the account's vacation reply, or clear it with `None`.
    ///
    /// Only the record of it: the reply itself runs on the server, and putting it there is a
    /// separate step that rewrites the account's script.
    fn put_vacation(
        &self,
        account: AccountId,
        vacation: Option<&Vacation>,
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

    /// What the user answered this message's request for a read receipt, if they have.
    fn receipt_answer(&self, message: MessageId) -> Result<Option<ReceiptAnswer>, StoreError>;

    /// Record the answer to a read-receipt request, and return the one that stands.
    ///
    /// The first answer stands and a later one is ignored: a request is put to the user once,
    /// and "declined, then sent anyway" is the thing recording it exists to prevent. Not undoable
    /// and not a [`Patch`] for the same reason [`Store::set_send_state`] is not — a receipt that
    /// left cannot be recalled. [`StoreError::NoMessage`] when the message is unknown.
    fn answer_receipt(
        &self,
        message: MessageId,
        answer: ReceiptAnswer,
        now: DateTime<Utc>,
    ) -> Result<ReceiptAnswer, StoreError>;

    /// What the user last answered the calendar invitation in this message, if they have.
    fn invite_answer(&self, message: MessageId) -> Result<Option<InviteAnswer>, StoreError>;

    /// Record an answer to the invitation in [`InviteAnswer::message`], replacing any earlier
    /// one: unlike a read receipt, a person may change their mind about a meeting, and each
    /// answer is a reply the organiser receives. Not a [`Patch`], for the reason
    /// [`Store::answer_receipt`] is not — the reply that left cannot be recalled.
    /// [`StoreError::NoMessage`] when the message is unknown.
    fn answer_invite(&self, answer: &InviteAnswer) -> Result<(), StoreError>;

    /// Every OpenPGP public key held — the user's own and correspondents' — by fingerprint.
    fn pgp_keys(&self) -> Result<Vec<PgpKey>, StoreError>;

    /// One key by fingerprint.
    fn pgp_key(&self, fingerprint: Fingerprint) -> Result<Option<PgpKey>, StoreError>;

    /// The keys for `address`, compared whole and without regard to case, best first: verified
    /// by the user, then by how the key arrived ([`mail_domain::KeySource::rank`]), then the
    /// most recently seen.
    fn pgp_keys_for(&self, address: &str) -> Result<Vec<PgpKey>, StoreError>;

    /// The keys a message could mean by `id` — a primary key's id or a subkey's — best first as
    /// [`Store::pgp_keys_for`] orders them.
    fn pgp_keys_by_id(&self, id: KeyId) -> Result<Vec<PgpKey>, StoreError>;

    /// Keep a key, folded into the record of the same key if there is one
    /// ([`PgpKey::merged`]), and return what is now stored.
    ///
    /// Not a [`Patch`]: a key is not mail, and keeping one is not an edit to undo.
    fn put_pgp_key(&self, key: PgpKey) -> Result<PgpKey, StoreError>;

    /// Mark a key verified by the user, or take that back. [`StoreError::NoPgpKey`] when there
    /// is no such key.
    fn set_pgp_trust(&self, fingerprint: Fingerprint, trust: KeyTrust) -> Result<(), StoreError>;

    /// Forget a key. `true` when there was one. Mail arriving later may bring it back.
    fn delete_pgp_key(&self, fingerprint: Fingerprint) -> Result<bool, StoreError>;

    /// The Autocrypt state kept for `address`, compared without regard to case.
    fn autocrypt_peer(&self, address: &str) -> Result<Option<AutocryptPeer>, StoreError>;

    /// Replace the Autocrypt state for the peer's address.
    fn put_autocrypt_peer(&self, peer: &AutocryptPeer) -> Result<(), StoreError>;

    /// Every S/MIME certificate held — the user's own, correspondents', issuers' — by
    /// fingerprint.
    fn smime_certs(&self) -> Result<Vec<SmimeCert>, StoreError>;

    /// One certificate by fingerprint.
    fn smime_cert(
        &self,
        fingerprint: mail_domain::CertFingerprint,
    ) -> Result<Option<SmimeCert>, StoreError>;

    /// The certificates for `address`, compared whole and without regard to case, best first:
    /// trusted by the user, then by how it arrived ([`mail_domain::CertSource::rank`]), then the
    /// one valid longest, then the most recently seen.
    fn smime_certs_for(&self, address: &str) -> Result<Vec<SmimeCert>, StoreError>;

    /// Keep a certificate, folded into the record of the same certificate if there is one
    /// ([`SmimeCert::merged`]), and return what is now stored.
    fn put_smime_cert(&self, cert: SmimeCert) -> Result<SmimeCert, StoreError>;

    /// Mark a certificate trusted by the user, or take that back. [`StoreError::NoSmimeCert`]
    /// when there is no such certificate.
    fn set_smime_trust(
        &self,
        fingerprint: mail_domain::CertFingerprint,
        trust: KeyTrust,
    ) -> Result<(), StoreError>;

    /// Forget a certificate. `true` when there was one. Mail arriving later may bring it back.
    fn delete_smime_cert(
        &self,
        fingerprint: mail_domain::CertFingerprint,
    ) -> Result<bool, StoreError>;

    /// Autocomplete: the best `k` contacts with a word beginning with `typed`, best first.
    ///
    /// A word is any word of the name, the address's local part or any word of it, the domain,
    /// or the whole address; `typed` is folded as search folds (case, diacritics), and when it
    /// is several words each must begin one — `ren mül` finds "Renée Müller". Empty `typed`
    /// matches everything.
    ///
    /// Only [`Contact::offered`] entries: never the user's own addresses, and never a list or
    /// no-reply sender the user has not written to or added. Ordered by frecency, writing to
    /// someone counting far more than hearing from them; ties by address.
    fn contacts_matching(&self, typed: &str, k: usize) -> Result<Vec<Contact>, StoreError>;

    /// One contact by address, in any case; `None` when there is none or it is not an address.
    fn contact(&self, address: &str) -> Result<Option<Contact>, StoreError>;

    /// Every contact, hidden ones included, in [`Store::contacts_matching`]'s order: for export,
    /// and for a sync deciding what it put in the book.
    fn contacts(&self) -> Result<Vec<Contact>, StoreError>;

    /// Add or edit a contact by hand, or from an address book.
    ///
    /// A `name` given is kept against anything mail says later. `None` leaves the current name;
    /// with [`Origin::History`] it also hands the name back to mail. Counts are kept either way.
    /// Returns the contact as stored; [`StoreError::BadAddress`] for something with no `@`.
    fn put_contact(
        &self,
        address: &str,
        name: Option<&str>,
        origin: &Origin,
    ) -> Result<Contact, StoreError>;

    /// Forget a contact. `true` when there was one. Mail arriving later may teach it again.
    fn delete_contact(&self, address: &str) -> Result<bool, StoreError>;

    /// Where the last sync of the address book at `url` got to.
    fn address_book(&self, url: &str) -> Result<Option<AddressBook>, StoreError>;

    /// Record where a sync of an address book got to, replacing what was there.
    fn put_address_book(&self, book: &AddressBook) -> Result<(), StoreError>;

    /// Every address book synced so far, by URL: what `mailo contacts sync` with no URL syncs.
    fn address_books(&self) -> Result<Vec<AddressBook>, StoreError>;
}
