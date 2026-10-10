//! What a test swaps in at the runtime's seams: an in-memory transport, account secrets that live
//! in a map, a signing store that does too, and accountd as a table.
//!
//! Behind the `testing` feature, which a crate enables for its tests and the program never does.
//! None of it reaches a socket, the platform's keyring or the session bus, so a test of the logic
//! above (or of a library built on this crate) needs no more than these to run in a sandbox.

use crate::account_secrets::AccountSecrets;
use crate::error::Failure;
use crate::link::{Accountd, Answer, Changes, LinkError};
use crate::signing_store::stored::Stored;
use crate::signing_store::{SigningStore, signing_entry_name, signing_secret, stored_signing};
use crate::{RuntimeError, Transport};
use mail_domain::signing::{SigningKeyRef, SigningSecret};
use porter_client::AuthenticatedStream;
use porter_core::stream::{DuplexEnd, duplex};
use porter_core::wire::Refusal;
use porter_core::{AccountId, Audience, Candidate, GrantId, IssuedToken, ServiceEndpoint};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

pub use porter_secrets::MemorySecrets;

/// A connection that goes nowhere: the [`Transport`] a machine is driven over, and the far end a
/// test plays the server on. The transport is a relay's (already secure, already signed in), so
/// the machine over it must not authenticate, which is what a test of the engines wants.
pub fn memory_transport() -> (Transport, DuplexEnd) {
    let (app, server) = duplex(64 * 1024);
    let transport = Transport::relayed(AuthenticatedStream::Memory(app))
        .expect("an in-memory stream needs no socket");
    (transport, server)
}

/// Account secrets that live in a map, empty to begin with.
pub fn memory_secrets() -> Arc<dyn AccountSecrets> {
    Arc::new(MemorySecrets::default())
}

/// A store of signing keys that lives in a map. It never reaches the user's keyring and never
/// persists.
#[derive(Debug, Default)]
pub struct MapSigningStore {
    entries: Mutex<HashMap<String, Stored>>,
}

impl SigningStore for MapSigningStore {
    fn get(&self, key: &SigningKeyRef) -> Result<SigningSecret, RuntimeError> {
        let held = self
            .entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .get(&signing_entry_name(key))
            .cloned()
            .ok_or_else(|| RuntimeError::Secrets(Failure::said("no such signing key")))?;
        signing_secret(held)
    }

    fn put(&self, key: &SigningKeyRef, value: &SigningSecret) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .insert(signing_entry_name(key), stored_signing(value));
        Ok(())
    }

    fn forget(&self, key: &SigningKeyRef) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .remove(&signing_entry_name(key));
        Ok(())
    }
}

/// What the add-account sheet answers with: the account it made, and the candidate Mail is
/// offered for it, if the sheet gave Mail a grant.
pub type Sheet = Result<(AccountId, Option<Candidate>), LinkError>;

/// accountd as a table: what it offers now, what its sheets answer, and what was asked of it.
///
/// Everything a test does not script is refused as "not part of this test", or, for the sheets,
/// as the person closing them. Build one with the `with_*` methods and hand it over as an
/// `Arc<dyn Accountd>`.
#[derive(Debug, Default)]
pub struct FakeAccountd {
    state: Mutex<Table>,
}

#[derive(Default)]
struct Table {
    offered: Vec<Candidate>,
    contacts: Vec<Candidate>,
    tokens: VecDeque<Result<IssuedToken, LinkError>>,
    adds: VecDeque<Sheet>,
    consents: VecDeque<Result<Candidate, LinkError>>,
    contact_consent: Option<Result<Candidate, LinkError>>,
    revoke_fault: Option<LinkError>,
    stream_fault: Option<LinkError>,
    feed: Option<Box<dyn Changes>>,
    asked: Vec<String>,
}

// By hand: the change feed has no `Debug` to rely on.
impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table").field("asked", &self.asked).finish()
    }
}

impl FakeAccountd {
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.state.lock().expect("FakeAccountd mutex poisoned")
    }

    /// Offers `candidates` as the accounts Mail holds a grant on.
    pub fn offering(candidates: Vec<Candidate>) -> Self {
        Self::default().with_offered(candidates)
    }

    /// Offers `candidates` as the accounts Mail holds a grant on.
    pub fn with_offered(self, candidates: Vec<Candidate>) -> Self {
        self.table().offered = candidates;
        self
    }

    /// Offers `candidates` as the accounts Mail holds a grant on for their contacts.
    pub fn with_contacts(self, candidates: Vec<Candidate>) -> Self {
        self.table().contacts = candidates;
        self
    }

    /// Answers the next token requests with `tokens`, in order; after them, as unreachable.
    pub fn with_tokens(self, tokens: Vec<Result<IssuedToken, LinkError>>) -> Self {
        self.table().tokens = tokens.into();
        self
    }

    /// Answers the next add-account sheets with `sheets`, in order; after them, as closed. An
    /// account that came with a grant is offered from then on.
    pub fn with_sheets(self, sheets: Vec<Sheet>) -> Self {
        self.table().adds = sheets.into();
        self
    }

    /// Answers the next consent sheets with `consents`, in order; after them, as closed. A
    /// candidate that was granted is offered from then on.
    pub fn with_consents(self, consents: Vec<Result<Candidate, LinkError>>) -> Self {
        self.table().consents = consents.into();
        self
    }

    /// Answers every request for a grant on contacts with `answer`.
    pub fn with_contact_consent(self, answer: Result<Candidate, LinkError>) -> Self {
        self.table().contact_consent = Some(answer);
        self
    }

    /// Fails every withdrawal of a grant with `fault`.
    pub fn with_revoke_refused(self, fault: LinkError) -> Self {
        self.table().revoke_fault = Some(fault);
        self
    }

    /// Fails every request for a raw stream with `fault`.
    pub fn with_stream_refused(self, fault: LinkError) -> Self {
        self.table().stream_fault = Some(fault);
        self
    }

    /// Hands `feed` to whoever follows the changes, once.
    pub fn with_changes(self, feed: impl Changes + 'static) -> Self {
        self.table().feed = Some(Box::new(feed));
        self
    }

    /// Queues one more add-account sheet answer, behind any already queued.
    pub fn push_sheet(&self, sheet: Sheet) {
        self.table().adds.push_back(sheet);
    }

    /// Queues one more consent sheet answer, behind any already queued.
    pub fn push_consent(&self, consent: Result<Candidate, LinkError>) {
        self.table().consents.push_back(consent);
    }

    /// Every call made so far, oldest first, each as its name (`candidates`, `token`,
    /// `request_grant`, ...), with the account or grant it was about where it had one.
    pub fn asked(&self) -> Vec<String> {
        self.table().asked.clone()
    }

    /// How many times `name` was asked.
    pub fn times_asked(&self, name: &str) -> usize {
        self.table().asked.iter().filter(|a| *a == name).count()
    }

    /// The grants withdrawn so far, in order.
    pub fn revoked(&self) -> Vec<String> {
        let prefix = "revoke ";
        self.table()
            .asked
            .iter()
            .filter_map(|a| a.strip_prefix(prefix))
            .map(str::to_owned)
            .collect()
    }
}

fn unscripted<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
}

impl Accountd for FakeAccountd {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        let mut table = self.table();
        table.asked.push("candidates".to_owned());
        let now = table.offered.clone();
        Box::pin(async move { Ok(now) })
    }

    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        let mut table = self.table();
        table.asked.push("token".to_owned());
        let next = table.tokens.pop_front();
        Box::pin(async move { next.unwrap_or(Err(LinkError::Unreachable)) })
    }

    fn open<'a>(&'a self, _: &'a GrantId, _: &'a ServiceEndpoint) -> Answer<'a, Transport> {
        self.table().asked.push("open".to_owned());
        unscripted()
    }

    fn open_stream<'a>(
        &'a self,
        _: &'a GrantId,
        _: &'a ServiceEndpoint,
    ) -> Answer<'a, AuthenticatedStream> {
        let mut table = self.table();
        table.asked.push("open_stream".to_owned());
        let fault = table.stream_fault.clone();
        Box::pin(async move {
            Err(fault.unwrap_or_else(|| LinkError::Other("not part of this test".to_owned())))
        })
    }

    fn contacts(&self) -> Answer<'_, Vec<Candidate>> {
        let mut table = self.table();
        table.asked.push("contacts".to_owned());
        let held = table.contacts.clone();
        Box::pin(async move { Ok(held) })
    }

    fn request_contacts(&self) -> Answer<'_, Candidate> {
        let mut table = self.table();
        table.asked.push("request_contacts".to_owned());
        let answer = table.contact_consent.clone();
        Box::pin(async move { answer.unwrap_or(Err(LinkError::Refused(Refusal::Dismissed))) })
    }

    fn add_account(&self) -> Answer<'_, AccountId> {
        let mut table = self.table();
        table.asked.push("add_account".to_owned());
        let next = table.adds.pop_front();
        // The account the sheet made is offered from now on, if it came with a grant.
        if let Some(Ok((_, Some(offer)))) = &next {
            table.offered.push(offer.clone());
        }
        Box::pin(async move {
            match next {
                Some(Ok((id, _))) => Ok(id),
                Some(Err(e)) => Err(e),
                None => Err(LinkError::Refused(Refusal::Dismissed)),
            }
        })
    }

    fn reauthenticate<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()> {
        self.table().asked.push(format!("reauthenticate {account}"));
        Box::pin(async { Ok(()) })
    }

    fn request_grant(&self) -> Answer<'_, Candidate> {
        let mut table = self.table();
        table.asked.push("request_grant".to_owned());
        let next = table.consents.pop_front();
        if let Some(Ok(offer)) = &next {
            table.offered.push(offer.clone());
        }
        Box::pin(async move { next.unwrap_or(Err(LinkError::Refused(Refusal::Dismissed))) })
    }

    fn revoke<'a>(&'a self, grant: &'a GrantId) -> Answer<'a, ()> {
        let mut table = self.table();
        table.asked.push(format!("revoke {grant}"));
        let fault = table.revoke_fault.clone();
        Box::pin(async move { fault.map_or(Ok(()), Err) })
    }

    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        let feed = self.table().feed.take();
        Box::pin(async move { Ok(feed) })
    }
}
