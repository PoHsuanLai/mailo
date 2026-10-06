//! Keeping an account signed in for as long as a process runs, and the seam engines get tokens
//! through.
//!
//! An access token lasts about an hour. [`TokenSource`] is what an engine asks before every
//! operation, and again when a server refuses a token that looked valid: "give me the credential
//! to present", "renew it once, because it was refused". The engine takes tokens from nowhere
//! else. [`OAuthTokens`] is the source that renews in process, through porter-oauth, over the
//! account's secrets (`porter_secrets`); at E6 another implementation answers from accountd's
//! `Accounts::token` and the engines do not change.
//!
//! It holds each of the account's access tokens to the issuer's schedule, not the pass's: the
//! incoming one in a [`Held`] cell that the backend's session factory reads each time it
//! connects, and, for an account that sends through Graph, the separate Graph token, which lives
//! in the account's secrets where [`TokenSource::current`] reads it.

use crate::AccountSecrets;
use crate::RuntimeError;
use crate::authorize::exchange_failure;
use crate::clients;
use chrono::{DateTime, Utc};
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, Retry, Retryable};
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose, SecretText, UnixSeconds};
use porter_oauth::{Renewal as Due, redeem_scope, refresh_scoped_detailed, renewal};
use porter_provider::ClientEntry;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

/// Where a source reads the time.
///
/// A function rather than an argument, which is the one place this runtime does that, because
/// the source outlives every call that could pass one: a token expires in wall-clock time while
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
    /// The token refused is one this source minted after a refusal already. A third token would
    /// be refused for the same reason, and asking for it is a loop against the issuer.
    StillRefused,
}

/// Where an engine gets the credential it presents.
///
/// The rule every engine follows (IMAP and SMTP `XOAUTH2`, Graph's bearer): [`ahead`] before an
/// operation; present [`current`]; if the server refuses the token (`NeedsReauth`), ask
/// [`after_refusal`] once and, on [`AfterRefusal::TryAgain`], run the operation once more; only
/// then report that the account needs signing in.
///
/// [`ahead`]: TokenSource::ahead
/// [`current`]: TokenSource::current
/// [`after_refusal`]: TokenSource::after_refusal
pub trait TokenSource: Send + Sync {
    /// Renew `token` if it is inside the refresh margin, before anything presents it. No network
    /// and no keyring when it is not.
    fn ahead<'a>(&'a self, token: Token) -> Answer<'a, ()>;

    /// A server refused `token`. Renew it once, unless this source already has.
    fn after_refusal<'a>(&'a self, token: Token) -> Answer<'a, AfterRefusal>;

    /// The credential to present for `token` now.
    fn current<'a>(&'a self, token: Token) -> Answer<'a, Credential>;
}

/// The incoming credential, shared between a source and whatever opens sessions with it.
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

fn unix(now: DateTime<Utc>) -> UnixSeconds {
    UnixSeconds(now.timestamp())
}

/// The refresh token to spend, when `credential` is an OAuth one that is due; none for a password,
/// which does not expire on a schedule, and for a token with time left.
fn due_refresh(credential: &Credential, now: DateTime<Utc>) -> Option<String> {
    match credential {
        Credential::OAuth {
            refresh,
            expires_at,
            ..
        } if renewal(*expires_at, unix(now)) == Due::Due => Some(refresh.expose().to_owned()),
        _ => None,
    }
}

/// Whether `credential` can be used as it stands.
pub fn is_fresh(credential: &Credential, now: DateTime<Utc>) -> bool {
    due_refresh(credential, now).is_none()
}

/// The scopes to name when renewing the access token the incoming server signs in with.
///
/// Empty when the sign-in was for one resource, which asks for what it was for: Google's
/// `mail.google.com`, or Exchange's IMAP and SMTP together. A sign-in that also consented to a
/// second resource (Graph's `Mail.Send`) must name the first one's scopes on every refresh:
/// Microsoft issues each access token for one resource and refuses a request that spans two
/// (`AADSTS28003`).
pub fn incoming_scopes(scopes: &[String]) -> Vec<String> {
    redeem_scope(scopes)
        .map(|chosen| chosen.split(' ').map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Spend `refresh_token` on a new access token, naming `scopes` when there are any. The refresh
/// token is carried through when the issuer does not rotate it: dropping it would log the user
/// out at the next expiry.
async fn refresh(
    client: &ClientEntry,
    refresh_token: &str,
    scopes: &[String],
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let endpoints = clients::endpoints(client)?;
    let http = crate::http::oauth_http()?;
    let presented = SecretText::new(refresh_token);
    let scope = (!scopes.is_empty()).then(|| scopes.join(" "));
    let tokens = refresh_scoped_detailed(&http, &endpoints, client, &presented, scope.as_deref())
        .await
        .map_err(|fault| {
            // Only the issuer's refusal (`invalid_grant`, a 401) is a revoked grant that no
            // amount of retrying brings back. An endpoint that could not be reached, or that
            // answered with something that is not OAuth, has refused nothing.
            exchange_failure("the issuer could not renew the sign-in", fault)
        })?;
    Ok(Credential::OAuth {
        expires_at: tokens.expires_at(unix(now)),
        refresh: tokens.refresh_token.unwrap_or(presented),
        access: tokens.access_token,
    })
}

/// Bring a credential up to date before it is used, storing whatever comes back.
///
/// Returns the credential to authenticate with. A password is returned untouched: this is the
/// one place that knows expiry is an OAuth concept, and making the caller ask first would put
/// that knowledge in every caller. `scopes` are the account plan's.
pub async fn renew(
    account: AccountId,
    client: &ClientEntry,
    scopes: &[String],
    credential: Credential,
    secrets: &dyn AccountSecrets,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let Some(refresh_token) = due_refresh(&credential, now) else {
        return Ok(credential);
    };
    renew_incoming(
        account,
        client,
        &incoming_scopes(scopes),
        &refresh_token,
        secrets,
        now,
    )
    .await
}

/// Spend `refresh_token` on a new incoming access token, whatever the old one's expiry said, and
/// keep it.
async fn renew_incoming(
    account: AccountId,
    client: &ClientEntry,
    scopes: &[String],
    refresh_token: &str,
    secrets: &dyn AccountSecrets,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let renewed = refresh(client, refresh_token, scopes, now).await?;
    // Both entries, because `account add` wrote both. The sync path reads `IncomingPassword`
    // and the browser flow is what writes `OAuthRefresh`; leaving either stale is the same
    // account failing an hour later, just via a different key. A refresh token the issuer
    // rotated is in `renewed`, so it is saved here too.
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        secrets
            .put(
                &SecretKey {
                    account: account.clone(),
                    purpose,
                },
                &renewed,
            )
            .await?;
    }
    Ok(renewed)
}

/// What an account's Graph token has to be good for, which decides the scopes it is minted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphReach {
    /// Sending only: an account that reads over IMAP. See [`mint_graph`] for why only
    /// `Mail.Send` is named.
    Send,
    /// Reading and sending: an account whose plan says [`mail_domain::Incoming::Graph`]. Its
    /// sign-in consented to `Mail.ReadWrite` because the preset asked for it, so naming it is
    /// safe, and one token then serves both directions.
    ReadAndSend,
}

impl GraphReach {
    /// What `plan` uses Graph for.
    pub fn of(plan: &AccountPlan) -> Self {
        match plan.incoming {
            Incoming::Graph => GraphReach::ReadAndSend,
            _ => GraphReach::Send,
        }
    }

    fn scopes(self) -> Vec<String> {
        let mut scopes = vec![mail_domain::presets::GRAPH_SEND_SCOPE.to_owned()];
        if self == GraphReach::ReadAndSend {
            scopes.insert(0, mail_domain::presets::GRAPH_WRITE_SCOPE.to_owned());
        }
        scopes.push("offline_access".to_owned());
        scopes
    }
}

/// A Graph access token for an account that sends through [`Outgoing::Graph`], or reads through
/// [`Incoming::Graph`].
///
/// Kept as the account's *outgoing* credential, beside the incoming one for Exchange's IMAP:
/// Microsoft's tokens are each for one resource, so an account that reads over IMAP and sends
/// through Graph holds two. Minted from the sign-in's refresh token the first time, and from its
/// own after that; a still-valid one is returned without touching the network.
pub async fn graph_token(
    account: AccountId,
    client: &ClientEntry,
    reach: GraphReach,
    secrets: &dyn AccountSecrets,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let held = secrets
        .get(&SecretKey {
            account: account.clone(),
            purpose: SecretPurpose::OutgoingPassword,
        })
        .await
        .ok();
    let refresh_token = match held {
        Some(held @ Credential::OAuth { .. }) => match due_refresh(&held, now) {
            None => return Ok(held),
            Some(refresh_token) => refresh_token,
        },
        // A password here is left over from an SMTP setup, and none at all is a first mint; the
        // sign-in's token is spent either way.
        _ => sign_in_refresh(secrets, account.clone()).await?,
    };
    mint_graph(account, client, reach, &refresh_token, secrets, now).await
}

/// The refresh token a new Graph token is minted from: the Graph token's own, or the sign-in's.
async fn graph_refresh(
    secrets: &dyn AccountSecrets,
    account: AccountId,
) -> Result<String, RuntimeError> {
    match secrets
        .get(&SecretKey {
            account: account.clone(),
            purpose: SecretPurpose::OutgoingPassword,
        })
        .await
    {
        Ok(Credential::OAuth { refresh, .. }) => Ok(refresh.expose().to_owned()),
        _ => sign_in_refresh(secrets, account).await,
    }
}

/// Spend `refresh_token` on a Graph token, and keep it as the account's outgoing credential.
///
/// Named with Graph's scope alone: one resource per request, or Microsoft refuses it.
///
/// For a sending-only account only `Mail.Send` is named, not `Mail.ReadWrite` too, although a
/// message over 4 MB needs it: Microsoft's token carries every permission already consented to
/// for the resource, not only the ones named, while naming one that was never consented to, as
/// a sign-in from before `send_through_graph` asked for it was not, would fail the refresh, and
/// with it every send. An account that reads through Graph has always asked for both.
async fn mint_graph(
    account: AccountId,
    client: &ClientEntry,
    reach: GraphReach,
    refresh_token: &str,
    secrets: &dyn AccountSecrets,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let minted = refresh(client, refresh_token, &reach.scopes(), now).await?;
    secrets
        .put(
            &SecretKey {
                account,
                purpose: SecretPurpose::OutgoingPassword,
            },
            &minted,
        )
        .await?;
    Ok(minted)
}

/// The refresh token the browser sign-in stored.
async fn sign_in_refresh(
    secrets: &dyn AccountSecrets,
    account: AccountId,
) -> Result<String, RuntimeError> {
    match secrets
        .get(&SecretKey {
            account,
            purpose: SecretPurpose::OAuthRefresh,
        })
        .await?
    {
        Credential::OAuth { refresh, .. } => Ok(refresh.expose().to_owned()),
        Credential::Password(_) | Credential::ApiKey(_) | Credential::KeyPair { .. } => Err(
            RuntimeError::Secrets("sending through Graph needs a Microsoft sign-in".to_owned()),
        ),
    }
}

/// What this source has already spent, so that nothing it does can become a loop.
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

/// One OAuth account's tokens, kept fresh in process through porter-oauth.
pub struct OAuthTokens {
    account: AccountId,
    address: String,
    client: ClientEntry,
    incoming_scopes: Vec<String>,
    outgoing: Outgoing,
    incoming: Incoming,
    secrets: Arc<dyn AccountSecrets>,
    held: Held,
    now: Now,
    spent: Mutex<Spent>,
}

// By hand: the client carries an application secret and the cell a credential.
impl std::fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("account", &self.account)
            .field("issuer", &self.client.issuer)
            .finish()
    }
}

impl OAuthTokens {
    /// A source for `plan`'s account, around the incoming credential in `held`.
    ///
    /// Reads the wall clock until told otherwise by [`OAuthTokens::with_clock`].
    pub fn new(
        account: AccountId,
        plan: &AccountPlan,
        client: ClientEntry,
        secrets: Arc<dyn AccountSecrets>,
        held: Held,
    ) -> Self {
        let incoming_scopes = match &plan.auth {
            AuthPlan::OAuth { scopes, .. } => incoming_scopes(scopes),
            AuthPlan::Password { .. } => Vec::new(),
        };
        Self {
            account,
            address: plan.address.clone(),
            client,
            incoming_scopes,
            outgoing: plan.outgoing.clone(),
            incoming: plan.incoming.clone(),
            secrets,
            held,
            now: Arc::new(Utc::now),
            spent: Mutex::new(Spent::default()),
        }
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

    async fn ahead_of(&self, token: Token) -> Result<(), RuntimeError> {
        let token = self.which(token);
        self.refused(token)?;
        let now = (self.now)();
        match token {
            Token::Incoming => {
                let Some(refresh_token) = due_refresh(&self.held.current(), now) else {
                    return Ok(());
                };
                self.incoming(&refresh_token, now).await.map(|_| ())
            }
            Token::Sending => {
                let minted = graph_token(
                    self.account.clone(),
                    &self.client,
                    self.reach(),
                    self.secrets.as_ref(),
                    now,
                )
                .await;
                self.judge(Token::Sending, minted).map(|_| ())
            }
        }
    }

    async fn after_refused(&self, token: Token) -> Result<AfterRefusal, RuntimeError> {
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
                if self.spent().incoming.as_deref() == Some(access.expose()) {
                    return Ok(AfterRefusal::StillRefused);
                }
                let renewed = self.incoming(refresh.expose(), now).await?;
                self.spent().incoming = access_of(&renewed);
            }
            Token::Sending => {
                let held = self.secrets.get(&self.sending_key()).await.ok();
                if let Some(Credential::OAuth { access, .. }) = &held
                    && self.spent().sending.as_deref() == Some(access.expose())
                {
                    return Ok(AfterRefusal::StillRefused);
                }
                let refresh_token =
                    graph_refresh(self.secrets.as_ref(), self.account.clone()).await?;
                let minted = mint_graph(
                    self.account.clone(),
                    &self.client,
                    self.reach(),
                    &refresh_token,
                    self.secrets.as_ref(),
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
        let renewed = renew_incoming(
            self.account.clone(),
            &self.client,
            &self.incoming_scopes,
            refresh_token,
            self.secrets.as_ref(),
            now,
        )
        .await;
        let renewed = self.judge(Token::Incoming, renewed)?;
        self.held.replace(renewed.clone());
        Ok(renewed)
    }

    /// Remember an issuer's refusal, and say it the way the user can act on.
    ///
    /// Anything else, an issuer that could not be reached, passes through as it is, and is
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

    fn reach(&self) -> GraphReach {
        match self.incoming {
            Incoming::Graph => GraphReach::ReadAndSend,
            _ => GraphReach::Send,
        }
    }

    fn sending_key(&self) -> SecretKey {
        SecretKey {
            account: self.account.clone(),
            purpose: SecretPurpose::OutgoingPassword,
        }
    }

    fn spent(&self) -> std::sync::MutexGuard<'_, Spent> {
        // Four `Option`s, each written whole: a poisoned lock still holds a coherent value.
        self.spent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl TokenSource for OAuthTokens {
    fn ahead<'a>(&'a self, token: Token) -> Answer<'a, ()> {
        Box::pin(self.ahead_of(token))
    }

    fn after_refusal<'a>(&'a self, token: Token) -> Answer<'a, AfterRefusal> {
        Box::pin(self.after_refused(token))
    }

    fn current<'a>(&'a self, token: Token) -> Answer<'a, Credential> {
        Box::pin(async move {
            match self.which(token) {
                Token::Incoming => Ok(self.held.current()),
                Token::Sending => self.secrets.get(&self.sending_key()).await,
            }
        })
    }
}

fn access_of(credential: &Credential) -> Option<String> {
    match credential {
        Credential::OAuth { access, .. } => Some(access.expose().to_owned()),
        Credential::Password(_) | Credential::ApiKey(_) | Credential::KeyPair { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_two_resource_sign_in_renews_the_incoming_token_with_the_first_resources_scopes() {
        assert_eq!(
            incoming_scopes(&scopes(&[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "offline_access",
                "openid",
                "https://graph.microsoft.com/Mail.Send",
            ])),
            scopes(&[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "offline_access",
                "openid"
            ])
        );
        // One resource: nothing is named, and the renewal asks for what the sign-in was for.
        assert!(
            incoming_scopes(&scopes(&[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
                "offline_access",
            ]))
            .is_empty()
        );
        assert!(incoming_scopes(&scopes(&["https://mail.google.com/", "email"])).is_empty());
    }

    #[test]
    fn a_token_is_due_inside_porters_margin_and_a_password_never_is() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let oauth = |seconds: i64| Credential::OAuth {
            access: SecretText::new("a"),
            refresh: SecretText::new("the-refresh-token"),
            expires_at: UnixSeconds(now.timestamp() + seconds),
        };
        assert!(is_fresh(&oauth(30 * 60), now));
        assert!(is_fresh(&oauth(61), now));
        assert_eq!(
            due_refresh(&oauth(60), now).as_deref(),
            Some("the-refresh-token")
        );
        // The state every OAuth account reached an hour after setup.
        assert!(!is_fresh(&oauth(-7200), now));
        assert!(is_fresh(&Credential::Password(SecretText::new("p")), now));
    }

    #[test]
    fn a_held_credential_debug_does_not_print_the_token() {
        let held = Held::new(Credential::OAuth {
            access: SecretText::new("ya29.secret-access"),
            refresh: SecretText::new("1//secret-refresh"),
            expires_at: UnixSeconds(0),
        });
        let rendered = format!("{held:?}");
        assert!(!rendered.contains("secret-access"), "{rendered}");
        assert!(!rendered.contains("secret-refresh"), "{rendered}");
    }
}
