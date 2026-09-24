//! Keeping an account signed in for as long as a process runs.
//!
//! An access token lives about an hour. `signin::renew` brings one up to date when a pass
//! starts, which is enough for `mailo sync` and not for `mailo watch`: an engine built once and
//! driven for a day kept the token it was built with, so an hour in every connection was refused
//! and the watch stopped on `NeedsReauth` with a perfectly good refresh token in the keyring.
//!
//! [`Renewal`] is what the engine asks before every operation, and again when a server refuses a
//! token that looked valid. It holds each of the account's access tokens to the issuer's
//! schedule, not the pass's: the incoming one in a [`Held`] cell that the backend's session
//! factory reads each time it connects, and — for an account that sends through Graph — the
//! separate Graph token, which lives in the keyring where submission already reads it.

use crate::RuntimeError;
use crate::oauth::{self, Freshness};
use crate::secrets::Secrets;
use crate::signin::{self, Registration};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, AccountPlan, AuthPlan, Credential, Incoming, Outgoing, Retry, Retryable,
};
use std::sync::{Arc, Mutex};

/// Where a [`Renewal`] reads the time.
///
/// A function rather than an argument, which is the one place this runtime does that, because
/// the renewal outlives every call that could pass one: a token expires in wall-clock time while
/// an `IDLE` sits parked or a first backfill runs for half an hour, and the `now` a pass began
/// with is stale by then. Injected, so a test fixes it, and a one-shot pass can hand in the
/// instant it was given.
pub type Now = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// Which of an account's access tokens an operation presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// The incoming server's: IMAP, and SMTP where the account submits over SMTP.
    Incoming,
    /// Microsoft Graph's, for an account whose plan says [`Outgoing::Graph`]. Anywhere else it
    /// is the incoming one, since that is what an SMTP submission authenticates with. An
    /// account that reads through Graph presents this one for [`Token::Incoming`] too.
    Sending,
}

/// What a second attempt is worth, after a server refused a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRefusal {
    /// A new token was minted; the operation should be tried once more with it.
    TryAgain,
    /// The token refused is one this renewal minted after a refusal already. A third token would
    /// be refused for the same reason, and asking for it is a loop against the issuer.
    StillRefused,
}

/// The incoming credential, shared between the renewal and whatever opens sessions with it.
///
/// A cell rather than a value because the backend's session factory is built once and called
/// on every connection: capturing the credential itself is how a watch came to present its first
/// token for ever.
#[derive(Debug, Clone)]
pub struct Held(Arc<Mutex<Credential>>);

impl Held {
    pub fn new(credential: Credential) -> Self {
        Self(Arc::new(Mutex::new(credential)))
    }

    /// The credential to sign in with now.
    pub fn current(&self) -> Credential {
        self.lock().clone()
    }

    fn replace(&self, credential: Credential) {
        *self.lock() = credential;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Credential> {
        // A poisoned cell still holds a whole credential: nothing panics while writing one.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What this renewal has already spent, so that nothing it does can become a loop.
#[derive(Debug, Default)]
struct Spent {
    /// The access token a refusal last minted, per token. Refused again, it is not renewed again.
    incoming: Option<String>,
    sending: Option<String>,
    /// The issuer's own refusal to renew, per token. It does not change within a process: the
    /// grant is revoked or expired, and only the user signing in again brings it back. Per token,
    /// because an administrator can withdraw consent to Graph's `Mail.Send` and leave IMAP be.
    refused_incoming: Option<String>,
    refused_sending: Option<String>,
}

/// One OAuth account's tokens, kept fresh.
pub struct Renewal {
    account: AccountId,
    address: String,
    registration: Registration,
    incoming_scopes: Vec<String>,
    outgoing: Outgoing,
    incoming: Incoming,
    secrets: Arc<dyn Secrets>,
    http: reqwest::Client,
    held: Held,
    now: Now,
    spent: Mutex<Spent>,
}

// By hand: the registration carries an application secret and the cell a credential.
impl std::fmt::Debug for Renewal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renewal")
            .field("account", &self.account)
            .field("issuer", &self.registration.issuer)
            .finish()
    }
}

impl Renewal {
    /// A renewal for `plan`'s account, around the incoming credential in `held`.
    ///
    /// Reads the wall clock until told otherwise by [`Renewal::with_clock`].
    pub fn new(
        account: AccountId,
        plan: &AccountPlan,
        registration: Registration,
        secrets: Arc<dyn Secrets>,
        held: Held,
    ) -> Result<Self, RuntimeError> {
        let incoming_scopes = match &plan.auth {
            AuthPlan::OAuth { scopes, .. } => oauth::incoming_scopes(scopes),
            AuthPlan::Password { .. } => Vec::new(),
        };
        Ok(Self {
            account,
            address: plan.address.clone(),
            registration,
            incoming_scopes,
            outgoing: plan.outgoing.clone(),
            incoming: plan.incoming.clone(),
            secrets,
            http: signin::http_client()?,
            held,
            now: Arc::new(Utc::now),
            spent: Mutex::new(Spent::default()),
        })
    }

    /// Read the time from `now` instead of the wall clock.
    pub fn with_clock(mut self, now: Now) -> Self {
        self.now = now;
        self
    }

    /// The incoming credential's cell, for the session factory to read.
    pub fn held(&self) -> Held {
        self.held.clone()
    }

    /// Renew `token` if it is inside the refresh margin, before anything presents it.
    ///
    /// No network and no keyring when it is not, which is every call but one an hour.
    pub async fn ahead(&self, token: Token) -> Result<(), RuntimeError> {
        let token = self.which(token);
        self.refused(token)?;
        let now = (self.now)();
        match token {
            Token::Incoming => {
                let refresh_token = match oauth::assess(&self.held.current(), now) {
                    Freshness::Ready => return Ok(()),
                    Freshness::Expired { refresh_token } => refresh_token.to_owned(),
                };
                self.incoming(&refresh_token, now).await.map(|_| ())
            }
            Token::Sending => {
                let minted = signin::graph_token(
                    self.account,
                    &self.registration,
                    self.reach(),
                    self.secrets.as_ref(),
                    &self.http,
                    now,
                )
                .await;
                self.judge(Token::Sending, minted).map(|_| ())
            }
        }
    }

    /// A server refused `token`. Renew it once, unless this renewal already has.
    pub async fn after_refusal(&self, token: Token) -> Result<AfterRefusal, RuntimeError> {
        let token = self.which(token);
        self.refused(token)?;
        let now = (self.now)();
        match token {
            Token::Incoming => {
                let Credential::OAuth {
                    access, refresh, ..
                } = self.held.current()
                else {
                    // A password is not renewed; its refusal is the user's to answer.
                    return Ok(AfterRefusal::StillRefused);
                };
                if self.spent().incoming.as_deref() == Some(access.as_str()) {
                    return Ok(AfterRefusal::StillRefused);
                }
                let renewed = self.incoming(&refresh, now).await?;
                self.spent().incoming = access_of(&renewed);
            }
            Token::Sending => {
                let held = self.secrets.get(&self.sending_key()).ok();
                if let Some(Credential::OAuth { access, .. }) = &held
                    && self.spent().sending.as_deref() == Some(access.as_str())
                {
                    return Ok(AfterRefusal::StillRefused);
                }
                let refresh_token = signin::graph_refresh(self.secrets.as_ref(), self.account)?;
                let minted = signin::mint_graph(
                    self.account,
                    &self.registration,
                    self.reach(),
                    &refresh_token,
                    self.secrets.as_ref(),
                    &self.http,
                    now,
                )
                .await;
                let minted = self.judge(Token::Sending, minted)?;
                self.spent().sending = access_of(&minted);
            }
        }
        Ok(AfterRefusal::TryAgain)
    }

    /// Renew the incoming token from `refresh_token`, and hand the result to the sessions.
    async fn incoming(
        &self,
        refresh_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Credential, RuntimeError> {
        let renewed = signin::renew_incoming(
            self.account,
            &self.registration,
            &self.incoming_scopes,
            refresh_token,
            self.secrets.as_ref(),
            &self.http,
            now,
        )
        .await;
        let renewed = self.judge(Token::Incoming, renewed)?;
        self.held.replace(renewed.clone());
        Ok(renewed)
    }

    /// Remember an issuer's refusal, and say it the way the user can act on.
    ///
    /// Anything else — an issuer that could not be reached — passes through as it is, and is
    /// tried again at the next operation.
    fn judge(
        &self,
        token: Token,
        outcome: Result<Credential, RuntimeError>,
    ) -> Result<Credential, RuntimeError> {
        let e = match outcome {
            Ok(credential) => return Ok(credential),
            Err(e) => e,
        };
        if !matches!(e.retry(), Retry::NeedsReauth) {
            return Err(e);
        }
        let why = format!(
            "{e}. Sign in again with: mailo account add {}",
            self.address
        );
        let mut spent = self.spent();
        match token {
            Token::Incoming => spent.refused_incoming = Some(why.clone()),
            Token::Sending => spent.refused_sending = Some(why.clone()),
        }
        Err(RuntimeError::Secrets(why))
    }

    /// The refusal already heard for `token`, if there was one, so it is not asked again.
    fn refused(&self, token: Token) -> Result<(), RuntimeError> {
        let spent = self.spent();
        let refused = match token {
            Token::Incoming => &spent.refused_incoming,
            Token::Sending => &spent.refused_sending,
        };
        match refused {
            Some(why) => Err(RuntimeError::Secrets(why.clone())),
            None => Ok(()),
        }
    }

    /// `token` as this account actually has it: Graph's own only where Graph is used.
    ///
    /// An account that reads through Graph presents Graph's token for everything, so its
    /// incoming token *is* the Graph one: one token, one resource, both directions.
    fn which(&self, token: Token) -> Token {
        match (token, &self.incoming, &self.outgoing) {
            (_, Incoming::Graph, _) => Token::Sending,
            (Token::Sending, _, Outgoing::Graph) => Token::Sending,
            _ => Token::Incoming,
        }
    }

    fn reach(&self) -> signin::GraphReach {
        match self.incoming {
            Incoming::Graph => signin::GraphReach::ReadAndSend,
            _ => signin::GraphReach::Send,
        }
    }

    fn sending_key(&self) -> mail_domain::SecretKey {
        mail_domain::SecretKey {
            account: self.account,
            purpose: mail_domain::SecretPurpose::OutgoingPassword,
        }
    }

    fn spent(&self) -> std::sync::MutexGuard<'_, Spent> {
        // Four `Option`s, each written whole: a poisoned lock still holds a coherent value.
        self.spent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn access_of(credential: &Credential) -> Option<String> {
    match credential {
        Credential::OAuth { access, .. } => Some(access.clone()),
        Credential::Password(_) | Credential::OpenPgp(_) => None,
    }
}
