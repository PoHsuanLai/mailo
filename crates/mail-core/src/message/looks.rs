//! What the reader finds out about one message beyond its body, and keeps so that opening it
//! again costs a lookup: the checks the receiving server made of the sender, the invitation it
//! carries, whether it asks for a read receipt, the thread's way out of its mailing list, and
//! what OpenPGP or S/MIME made of it ([`seal`](super::seal)).
//!
//! Each of these is in the stored raw message, so finding it out reads a blob, and sometimes
//! decrypts. They run on a blocking thread, for a message the reader has open and nowhere else,
//! and the answer is kept per message and body, so a body arriving asks again and one message's
//! answer is never given for another. A message whose body is not here has no answer: fetching
//! it to find out would be a POP3 `RETR`, which marks it read.
//!
//! The answers are typed. The words for them, and the card an invitation is drawn as, are the
//! front-end's. [`Looks`] is a value the front-end owns and hands to whoever reads, not a
//! process global: two of them share nothing. [`Looks::reads`] counts how often each lookup
//! really read, so a test can say that a path did not.

use super::list::{Offer, offer_of};
use super::seal::{self, Kept};
use crate::invite::InviteState;
use crate::receipt::ReceiptState;
use crate::recent::Recent;
use mail_domain::*;
use mail_mime::AuthResults;
use mail_pim::Invite;
use mail_store::{SqliteStore, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// How many messages' checks to keep. An answer is a few short strings.
const CHECKS: usize = 128;
/// How many messages' invitations to keep. Each is a few short strings.
const INVITES: usize = 128;
/// How many messages' read receipt standings to keep. Each is a few short strings.
const RECEIPTS: usize = 256;
/// How many threads' list offers to keep. An offer is a few short strings.
const LISTS: usize = 64;

/// What a thread's answers are a function of: its messages and the bytes each one has. A body
/// that arrives, or a message that joins the thread, is a new key; nothing else changes it,
/// because a blob is content-addressed.
pub type Bodies = Vec<(MessageId, Option<BlobId>)>;

/// Which lookup, for [`Looks::reads`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// A sender's checks, read from the stored raw message.
    Checks,
    /// A message's invitation.
    Invite,
    /// Whether a message asks for a read receipt.
    Receipt,
    /// A thread's way out of its list.
    List,
    /// A message opened as OpenPGP or S/MIME.
    Seal,
}

const READS: usize = 5;

/// An invitation a message carries, with what the user last answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invited {
    pub invite: Box<Invite>,
    pub answered: Option<InviteAnswer>,
}

/// One message's standing on read receipts, with the name the front-end calls its sender by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub message: MessageId,
    /// The sender's name, else their address.
    pub sender: String,
    pub state: ReceiptState,
}

/// The reader's per-message lookups and what they found. One for the whole app.
pub struct Looks {
    checks: Mutex<Recent<MessageId, (BlobId, Option<AuthResults>)>>,
    invites: Mutex<Recent<MessageId, (Option<BlobId>, Option<Invited>)>>,
    receipts: Mutex<Recent<MessageId, (Option<BlobId>, Option<Receipt>)>>,
    lists: Mutex<Recent<ThreadId, (Bodies, Option<Offer>)>>,
    pub(super) seals: Mutex<Recent<MessageId, Kept>>,
    reads: [AtomicUsize; READS],
}

/// The memory under `lock`, whether or not a thread panicked holding it: what it holds is only
/// ever a whole answer.
pub(super) fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for Looks {
    fn default() -> Self {
        Looks::new()
    }
}

impl Looks {
    /// A fresh set of lookups, with nothing found yet.
    pub fn new() -> Self {
        Looks {
            checks: Mutex::new(Recent::new(CHECKS)),
            invites: Mutex::new(Recent::new(INVITES)),
            receipts: Mutex::new(Recent::new(RECEIPTS)),
            lists: Mutex::new(Recent::new(LISTS)),
            seals: Mutex::new(Recent::new(seal::KEPT)),
            reads: Default::default(),
        }
    }

    /// How many times `read` has read the store (not asked the memory) through this value.
    pub fn reads(&self, read: Read) -> usize {
        self.reads[read as usize].load(Ordering::Relaxed)
    }

    pub(super) fn count(&self, read: Read) {
        self.reads[read as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// The answer already found for `message` with body `raw`, if one has been. `Some(None)` is
    /// a message that was read and has no field this client believes. Never reads the store, so
    /// the thread that draws may ask.
    pub fn checks_cached(&self, message: MessageId, raw: BlobId) -> Option<Option<AuthResults>> {
        held(&self.checks)
            .get(&message)
            .filter(|(body, _)| *body == raw)
            .map(|(_, results)| results.clone())
    }

    /// [`checks_cached`](Self::checks_cached), else read from the stored bytes and remembered.
    /// Blocking: run it on a blocking thread.
    pub fn checks(
        &self,
        store: &SqliteStore,
        message: MessageId,
        raw: BlobId,
    ) -> Option<AuthResults> {
        if let Some(had) = self.checks_cached(message, raw) {
            return had;
        }
        self.count(Read::Checks);
        let results = store
            .message(message)
            .ok()
            .and_then(|message| crate::auth::results_of(store, &message));
        held(&self.checks).put(message, (raw, results.clone()));
        results
    }

    /// The invitation already found for `message` holding `body`, if it has been looked at.
    /// `Some(None)` is a message that was looked at and invites to nothing.
    pub fn invite_cached(
        &self,
        message: MessageId,
        body: Option<BlobId>,
    ) -> Option<Option<Invited>> {
        held(&self.invites)
            .get(&message)
            .filter(|(at, _)| *at == body)
            .map(|(_, invited)| invited.clone())
    }

    /// [`invite_cached`](Self::invite_cached), else read from the store and remembered. `None`
    /// when the message carries no invitation, or only its headers are here. This reads a blob:
    /// run it on a blocking thread, for a message the reader has open.
    pub fn invite(
        &self,
        store: &SqliteStore,
        message: MessageId,
        body: Option<BlobId>,
    ) -> Option<Invited> {
        if let Some(had) = self.invite_cached(message, body) {
            return had;
        }
        let invited = self.read_invite(store, message);
        held(&self.invites).put(message, (body, invited.clone()));
        invited
    }

    /// The invitation as the store holds it now, kept under the body the message has: for after
    /// an answer, so the reader shows what the store now holds.
    pub fn invite_again(&self, store: &SqliteStore, message: MessageId) -> Option<Invited> {
        let invited = self.read_invite(store, message);
        if let Ok(stored) = store.message(message) {
            held(&self.invites).put(message, (stored.body.raw(), invited.clone()));
        }
        invited
    }

    fn read_invite(&self, store: &SqliteStore, message: MessageId) -> Option<Invited> {
        self.count(Read::Invite);
        let stored = store.message(message).ok()?;
        match crate::invite::state(store, &stored).ok()? {
            InviteState::Shown { invite, answered } => Some(Invited { invite, answered }),
            InviteState::NotInvite | InviteState::Unknown => None,
        }
    }

    /// The standings already found for every message in `key`, or `None` if any is missing.
    pub fn receipts_cached(&self, key: &[(MessageId, Option<BlobId>)]) -> Option<Vec<Receipt>> {
        let memory = held(&self.receipts);
        key.iter()
            .map(|(message, body)| {
                memory
                    .get(message)
                    .filter(|(at, _)| at == body)
                    .map(|(_, standing)| standing.clone())
            })
            .collect::<Option<Vec<_>>>()
            .map(|all| all.into_iter().flatten().collect())
    }

    /// [`receipts_cached`](Self::receipts_cached), else read for each message and remembered.
    /// Blocking: run it on a blocking thread.
    pub fn receipts(
        &self,
        store: &SqliteStore,
        key: &[(MessageId, Option<BlobId>)],
    ) -> Vec<Receipt> {
        if let Some(had) = self.receipts_cached(key) {
            return had;
        }
        key.iter()
            .filter_map(|(message, body)| {
                let standing = self.read_receipt(store, *message);
                held(&self.receipts).put(*message, (*body, standing.clone()));
                standing
            })
            .collect()
    }

    /// The standing as the store holds it now, kept under the body the message has: for after an
    /// answer, so the bar settles without reading the store again.
    pub fn receipt_again(&self, store: &SqliteStore, message: MessageId) {
        if let Ok(stored) = store.message(message) {
            let standing = self.read_receipt(store, message);
            held(&self.receipts).put(message, (stored.body.raw(), standing));
        }
    }

    fn read_receipt(&self, store: &SqliteStore, message: MessageId) -> Option<Receipt> {
        self.count(Read::Receipt);
        let stored = store.message(message).ok()?;
        let state = crate::receipt::state(store, &stored).ok()?;
        let sender = stored
            .from
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| stored.from.email.clone());
        Some(Receipt {
            message,
            sender,
            state,
        })
    }

    /// The answer already found for `thread` at `key`, if one has been. `Some(None)` is a thread
    /// that was looked at and offers nothing.
    pub fn offer_cached(&self, thread: ThreadId, key: &Bodies) -> Option<Option<Offer>> {
        held(&self.lists)
            .get(&thread)
            .filter(|(at, _)| at == key)
            .map(|(_, offer)| offer.clone())
    }

    /// [`offer_cached`](Self::offer_cached), else read from the stored mail and remembered. This
    /// reads a blob: run it on a blocking thread, for the thread the reader has open, and never
    /// from the list's rows.
    pub fn offer(&self, store: &SqliteStore, thread: ThreadId, key: &Bodies) -> Option<Offer> {
        if let Some(had) = self.offer_cached(thread, key) {
            return had;
        }
        let offer = self.read_offer(store, thread);
        held(&self.lists).put(thread, (key.clone(), offer.clone()));
        offer
    }

    fn read_offer(&self, store: &SqliteStore, thread: ThreadId) -> Option<Offer> {
        self.count(Read::List);
        // Only stored bytes are read: a message whose body is not here is skipped, never fetched.
        let found = crate::unsubscribe::find(store, *thread.as_uuid()).ok()?;
        let sender = store.message(found.message).ok()?.from;
        let from =
            crate::compose::address_addressed(store, found.account.clone(), &found.addressed)
                .ok()?;
        offer_of(found, sender, from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown() -> (MessageId, BlobId) {
        (
            MessageId::from_uuid(uuid::Uuid::from_u128(1)),
            BlobId::from_uuid(uuid::Uuid::from_u128(2)),
        )
    }

    #[test]
    fn an_answer_found_is_kept_and_the_second_ask_does_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let looks = Looks::new();
        let (message, raw) = unknown();
        assert_eq!(looks.checks_cached(message, raw), None);
        assert_eq!(looks.checks(&store, message, raw), None);
        assert_eq!(looks.checks_cached(message, raw), Some(None));
        assert_eq!(looks.checks(&store, message, raw), None);
        assert_eq!(looks.reads(Read::Checks), 1);
        assert_eq!(looks.reads(Read::Invite), 0);
    }

    #[test]
    fn a_body_that_arrives_is_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let looks = Looks::new();
        let (message, _) = unknown();
        assert_eq!(looks.invite(&store, message, None), None);
        assert_eq!(looks.invite_cached(message, None), Some(None));
        let body = Some(BlobId::from_uuid(uuid::Uuid::from_u128(3)));
        assert_eq!(looks.invite_cached(message, body), None);
        assert_eq!(looks.invite(&store, message, body), None);
        assert_eq!(looks.reads(Read::Invite), 2);
    }

    #[test]
    fn two_looks_share_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let (one, other) = (Looks::new(), Looks::new());
        let (message, raw) = unknown();
        one.checks(&store, message, raw);
        assert_eq!(other.checks_cached(message, raw), None);
        assert_eq!(other.reads(Read::Checks), 0);
    }
}
