//! [`TokenSource`] over accountd: `Accounts::token`, asked again when the token is near its end
//! and once after a server refused one.
//!
//! The same rule every engine follows (`tokens`): [`TokenSource::ahead`] before an operation,
//! present [`TokenSource::current`], and after a refusal [`TokenSource::after_refusal`] once. What
//! differs from the in-process source is where the token comes from: accountd holds the refresh
//! token and mailo never sees it, so the credential this hands an engine is an OAuth credential
//! whose refresh half is empty. A refusal that is accountd's own (`NeedsReauth`, the grant gone)
//! is remembered and said again without asking, as the in-process source does for the issuer's.

use super::{Accountd, LinkError, fresh};
use crate::RuntimeError;
use crate::tokens::{AfterRefusal, Now, Token, TokenSource};
use chrono::Utc;
use porter_core::{Audience, Credential, GrantId, IssuedToken, SecretText, UnixSeconds};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

#[derive(Debug, Default)]
struct State {
    /// The token in hand.
    held: Option<IssuedToken>,
    /// The access token a refusal last minted: refused again, it is not renewed again.
    spent: Option<String>,
    /// accountd's own refusal, said again until the process is asked to sign in again.
    refused: Option<LinkError>,
}

/// One linked account's token for one audience (`Graph`'s, `JMAP`'s).
pub struct LinkedTokens {
    link: Arc<dyn Accountd>,
    grant: GrantId,
    audience: Audience,
    now: Now,
    state: Mutex<State>,
}

impl std::fmt::Debug for LinkedTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkedTokens")
            .field("grant", &self.grant)
            .field("audience", &self.audience)
            .finish_non_exhaustive()
    }
}

impl LinkedTokens {
    /// Tokens on `grant` for `audience`, reading the wall clock until [`LinkedTokens::with_clock`].
    pub fn new(link: Arc<dyn Accountd>, grant: GrantId, audience: Audience) -> Self {
        Self {
            link,
            grant,
            audience,
            now: Arc::new(Utc::now),
            state: Mutex::new(State::default()),
        }
    }

    /// Read the time from `now`.
    pub fn with_clock(mut self, now: Now) -> Self {
        self.now = now;
        self
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // Plain values written whole: a poisoned lock still holds a coherent state.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn unix(&self) -> UnixSeconds {
        UnixSeconds((self.now)().timestamp())
    }

    fn said(&self) -> Result<(), RuntimeError> {
        match &self.state().refused {
            Some(refusal) => Err(refusal.clone().into()),
            None => Ok(()),
        }
    }

    /// Ask accountd, keep the answer, and remember a refusal that only the person can answer.
    async fn mint(&self) -> Result<IssuedToken, RuntimeError> {
        match self.link.token(&self.grant, &self.audience).await {
            Ok(token) => {
                self.state().held = Some(token.clone());
                Ok(token)
            }
            Err(error) => {
                if error.needs_person() {
                    self.state().refused = Some(error.clone());
                }
                Err(error.into())
            }
        }
    }

    async fn ahead_of(&self) -> Result<(), RuntimeError> {
        self.said()?;
        let now = self.unix();
        if self.state().held.as_ref().is_some_and(|t| fresh(t, now)) {
            return Ok(());
        }
        self.mint().await.map(|_| ())
    }

    async fn after_refused(&self) -> Result<AfterRefusal, RuntimeError> {
        self.said()?;
        let spent = {
            let state = self.state();
            match (&state.held, &state.spent) {
                (Some(held), Some(spent)) => held.value.expose() == spent,
                _ => false,
            }
        };
        if spent {
            return Ok(AfterRefusal::StillRefused);
        }
        let minted = self.mint().await?;
        self.state().spent = Some(minted.value.expose().to_owned());
        Ok(AfterRefusal::TryAgain)
    }

    async fn current_token(&self) -> Result<Credential, RuntimeError> {
        self.said()?;
        let now = self.unix();
        let held = self.state().held.clone().filter(|t| fresh(t, now));
        let token = match held {
            Some(token) => token,
            None => self.mint().await?,
        };
        Ok(Credential::OAuth {
            access: SecretText::new(token.value.expose()),
            // The refresh token never leaves accountd.
            refresh: SecretText::new(""),
            expires_at: token.expires,
        })
    }
}

impl TokenSource for LinkedTokens {
    fn ahead<'a>(&'a self, _token: Token) -> Answer<'a, ()> {
        Box::pin(self.ahead_of())
    }

    fn after_refusal<'a>(&'a self, _token: Token) -> Answer<'a, AfterRefusal> {
        Box::pin(self.after_refused())
    }

    fn current<'a>(&'a self, _token: Token) -> Answer<'a, Credential> {
        Box::pin(self.current_token())
    }
}
