//! [`Accountd`] over a porter-client transport.

use super::{Accountd, Answer, Change, Changes, LinkError};
use crate::Transport;
use porter_client::{
    Accounts, AuthenticatedStream, ClientError, Found, Transport as Carrier, TransportError,
};
use porter_core::capability::{Access, Delta, Offered};
use porter_core::consent::Usage;
use porter_core::need::{MailNeed, PimNeed};
use porter_core::wire::{ParentWindow, ProviderHint};
use porter_core::{
    AccountId, Audience, Candidate, DataClass, GrantId, IssuedToken, Need, ServiceEndpoint,
};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

/// Makes a feed of changes when asked (what the bus link follows the signals with).
type Feed = Box<dyn Fn() -> Answer<'static, Option<Box<dyn Changes>>> + Send + Sync>;

/// Mail's questions to accountd, over `T`: the bus in the desktop, an in-process service in a test.
pub struct Client<T> {
    accounts: Accounts<T>,
    usage: Usage,
    /// The candidates last read, by grant: a token is asked for by the candidate that holds the
    /// grant (`Accounts::token` takes one), and the plan keeps only the grant.
    known: Mutex<HashMap<GrantId, Candidate>>,
    /// What to follow, when the transport has a feed (`Accountd::changes`).
    feed: Option<Feed>,
}

impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

impl<T: Carrier + 'static> Client<T> {
    /// Over `accounts`, for a process that uses its grants as `usage`.
    pub fn new(accounts: Accounts<T>, usage: Usage) -> Self {
        Self {
            accounts,
            usage,
            known: Mutex::new(HashMap::new()),
            feed: None,
        }
    }

    /// With a feed of changes to follow (the bus's signals).
    pub fn with_changes(
        mut self,
        feed: impl Fn() -> Answer<'static, Option<Box<dyn Changes>>> + Send + Sync + 'static,
    ) -> Self {
        self.feed = Some(Box::new(feed));
        self
    }

    /// What Mail needs of an account: to read and write its mail. Sending is not required, so an
    /// account that only receives is one (the engine says so when it is asked to send).
    fn need() -> Need {
        Need::Mail(MailNeed {
            access: Access::ReadWrite,
            send: Offered::Absent,
            delta: Delta::None,
        })
    }

    /// What Mail needs of an account to read its address books: reading them. Writing a group back
    /// is tried when a group was edited, and a server that refuses it says so then.
    fn contacts_need() -> Need {
        Need::Contacts(PimNeed {
            access: Access::Read,
            delta: Delta::None,
        })
    }

    fn remember(&self, candidates: &[Candidate]) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        for candidate in candidates {
            known.insert(candidate.grant.clone(), candidate.clone());
        }
    }

    async fn candidate(&self, grant: &GrantId) -> Result<Candidate, LinkError> {
        let held = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(grant)
            .cloned();
        if let Some(candidate) = held {
            return Ok(candidate);
        }
        // Not read yet in this process (a plan stored by an earlier run): read them.
        self.read().await?;
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(grant)
            .cloned()
            .ok_or(LinkError::Refused(porter_core::wire::Refusal::UnknownGrant))
    }

    async fn read(&self) -> Result<Vec<Candidate>, LinkError> {
        let found = self
            .accounts
            .find(&Self::need(), DataClass::Mail, self.usage)
            .await
            .map_err(failed)?;
        let candidates = match found {
            Found::One(one) => vec![one],
            Found::Several(many) => many,
            // Accounts exist and Mail has no grant on them yet, or there are none: nothing to read.
            Found::NeedsConsent(_) | Found::None(_) => Vec::new(),
        };
        // What was known and is not any more is forgotten with it, or a revoked grant would go on
        // being asked for.
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        known.clear();
        for candidate in &candidates {
            known.insert(candidate.grant.clone(), candidate.clone());
        }
        Ok(candidates)
    }
}

/// A client failure as the link's.
pub(crate) fn failed(error: ClientError) -> LinkError {
    match error {
        ClientError::Transport(TransportError::Unreachable | TransportError::Closed) => {
            LinkError::Unreachable
        }
        ClientError::Transport(other) => LinkError::Other(other.to_string()),
        ClientError::Refused(refusal) => LinkError::Refused(refusal),
        // A reply that does not answer the question (a daemon of another version), and whatever
        // else a client may say with the features it has.
        other => LinkError::Other(other.to_string()),
    }
}

impl<T: Carrier + 'static> Accountd for Client<T> {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        Box::pin(self.read())
    }

    fn token<'a>(&'a self, grant: &'a GrantId, audience: &'a Audience) -> Answer<'a, IssuedToken> {
        Box::pin(async move {
            let candidate = self.candidate(grant).await?;
            self.accounts
                .token(&candidate, audience)
                .await
                .map_err(failed)
        })
    }

    fn open<'a>(
        &'a self,
        grant: &'a GrantId,
        endpoint: &'a ServiceEndpoint,
    ) -> Answer<'a, Transport> {
        Box::pin(async move {
            let stream = self
                .accounts
                .open_authenticated(grant, &endpoint.url)
                .await
                .map_err(failed)?;
            Transport::relayed(stream).map_err(|e| LinkError::Other(e.to_string()))
        })
    }

    fn open_stream<'a>(
        &'a self,
        grant: &'a GrantId,
        endpoint: &'a ServiceEndpoint,
    ) -> Answer<'a, AuthenticatedStream> {
        Box::pin(async move {
            self.accounts
                .open_authenticated(grant, &endpoint.url)
                .await
                .map_err(failed)
        })
    }

    fn contacts(&self) -> Answer<'_, Vec<Candidate>> {
        Box::pin(async move {
            let found = self
                .accounts
                .find(&Self::contacts_need(), DataClass::Contacts, self.usage)
                .await
                .map_err(failed)?;
            Ok(match found {
                Found::One(one) => vec![one],
                Found::Several(many) => many,
                // No grant yet (or no account): the caller asks, with `request_contacts`.
                Found::NeedsConsent(_) | Found::None(_) => Vec::new(),
            })
        })
    }

    fn request_contacts(&self) -> Answer<'_, Candidate> {
        Box::pin(async move {
            let offer = porter_client::ConsentOffer {
                need: Self::contacts_need(),
                class: DataClass::Contacts,
                usage: self.usage,
            };
            self.accounts
                .request_grant(&offer, &ParentWindow::Unparented)
                .await
                .map_err(failed)
        })
    }

    fn add_account(&self) -> Answer<'_, AccountId> {
        Box::pin(async move {
            self.accounts
                .add_account(ProviderHint::Any, &ParentWindow::Unparented)
                .await
                .map_err(failed)
        })
    }

    fn reauthenticate<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()> {
        Box::pin(async move {
            self.accounts
                .reauthenticate(account, &ParentWindow::Unparented)
                .await
                .map_err(failed)
        })
    }

    fn request_grant(&self) -> Answer<'_, Candidate> {
        Box::pin(async move {
            let offer = porter_client::ConsentOffer {
                need: Self::need(),
                class: DataClass::Mail,
                usage: self.usage,
            };
            let candidate = self
                .accounts
                .request_grant(&offer, &ParentWindow::Unparented)
                .await
                .map_err(failed)?;
            self.remember(std::slice::from_ref(&candidate));
            Ok(candidate)
        })
    }

    fn revoke<'a>(&'a self, grant: &'a GrantId) -> Answer<'a, ()> {
        Box::pin(async move {
            self.accounts.revoke(grant).await.map_err(failed)?;
            self.known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(grant);
            Ok(())
        })
    }

    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        match &self.feed {
            Some(feed) => feed(),
            None => Box::pin(async { Ok(None) }),
        }
    }
}

/// A feed that is a list: what a test follows.
#[derive(Debug)]
pub struct Listed(pub std::collections::VecDeque<Change>);

impl Changes for Listed {
    fn next(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Change>> + Send + '_>> {
        Box::pin(async move { self.0.pop_front() })
    }
}
