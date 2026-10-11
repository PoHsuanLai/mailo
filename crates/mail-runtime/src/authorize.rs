//! The browser sign-in: PKCE, a loopback redirect, and the exchange of the code.
//!
//! The rules (the authorize URL, what a redirect may be, the exchange) are porter-oauth's; this
//! is the part that needs a socket and randomness, which porter-oauth takes as arguments.

use crate::RuntimeError;
use crate::error::Failure;
use porter_core::{Credential, UnixSeconds};
use porter_oauth::{
    ExchangeFailure, ExchangeFault, LoopbackFault, LoopbackServer, Pkce, RedirectPath,
    authorize_url, exchange_code_scoped_detailed, redeem_scope,
};
use porter_provider::{ClientEntry, Issuer};
use std::time::Duration;

/// Give up if no browser arrives. The user may have closed the tab or never signed in.
const WAIT: Duration = Duration::from_secs(300);

/// What a failed exchange with an issuer means. Only the issuer's own refusal is `Secrets`, which
/// reads as `NeedsReauth`: an issuer that cannot be reached this minute has refused nothing, and
/// calling it a rejected sign-in would stop a watch that only had to wait.
pub(crate) fn exchange_fault(what: &str, fault: ExchangeFault) -> RuntimeError {
    match fault {
        ExchangeFault::Refused => {
            RuntimeError::Secrets(Failure::said(format!("{what}: the issuer refused it")))
        }
        ExchangeFault::Unreachable => {
            RuntimeError::Connect(Failure::said(format!("{what}: token endpoint unreachable")))
        }
        ExchangeFault::Unreadable => RuntimeError::Connect(Failure::said(format!(
            "{what}: the token endpoint answered unreadably"
        ))),
    }
}

/// [`exchange_fault`] for a `*_detailed` call: the same meaning, and the issuer's own
/// `error_description` (its `error` code when it gave none) after the fault, as mailo always showed it.
pub(crate) fn exchange_failure(what: &str, failure: ExchangeFailure) -> RuntimeError {
    let said = failure
        .says
        .and_then(|says| says.description.or(says.error))
        .filter(|text| !text.is_empty());
    let mut error = exchange_fault(what, failure.fault);
    if let Some(said) = said
        && let RuntimeError::Secrets(failure) | RuntimeError::Connect(failure) = &mut error
    {
        failure.add_detail(&said);
    }
    error
}

/// Sign in as `client` in the user's browser and return the credential.
///
/// Binds first: the redirect URI has to name the port actually got. `on_url` is handed the
/// address to open before the wait begins.
pub async fn sign_in(
    client: &ClientEntry,
    scopes: &[String],
    on_url: &dyn Fn(&str),
    now: UnixSeconds,
) -> Result<Credential, RuntimeError> {
    sign_in_within(client, scopes, on_url, now, WAIT).await
}

/// [`sign_in`] giving up on the browser after `wait`. Cancel by dropping the future: the
/// listener goes with it.
pub async fn sign_in_within(
    client: &ClientEntry,
    scopes: &[String],
    on_url: &dyn Fn(&str),
    now: UnixSeconds,
    wait: Duration,
) -> Result<Credential, RuntimeError> {
    sign_in_told(client, scopes, on_url, now, wait)
        .await
        .map_err(RuntimeError::from)
}

/// How a browser sign-in ended without a credential, told apart where a front end words them
/// differently: the person said no, nobody came back, or something else stopped it.
#[derive(Debug, thiserror::Error)]
pub enum SignInFault {
    /// The issuer sent the browser back with an error: the person denied the sign-in or closed
    /// it on the issuer's page. What the issuer said.
    #[error("authorization declined: {0}")]
    Declined(String),
    /// No browser came back before the wait ran out.
    #[error("the sign-in did not complete: {}", LoopbackFault::TimedOut)]
    TimedOut,
    /// The browser came back with something that was not this sign-in's answer.
    #[error("the sign-in did not complete: {0}")]
    Unfinished(LoopbackFault),
    /// The listener, the exchange or the issuer's answer failed.
    #[error(transparent)]
    Failed(RuntimeError),
}

impl From<SignInFault> for RuntimeError {
    /// As [`sign_in_within`] has always said it: every fault is the issuer's or the sign-in's
    /// (`Secrets`), except what failed on its own.
    fn from(fault: SignInFault) -> Self {
        match fault {
            SignInFault::Declined(why) => {
                RuntimeError::Secrets(Failure::new("authorization declined", why))
            }
            SignInFault::TimedOut => RuntimeError::Secrets(Failure::new(
                "the sign-in did not complete",
                LoopbackFault::TimedOut,
            )),
            SignInFault::Unfinished(other) => {
                RuntimeError::Secrets(Failure::new("the sign-in did not complete", other))
            }
            SignInFault::Failed(error) => error,
        }
    }
}

/// [`sign_in_within`], with how it ended told apart ([`SignInFault`]).
pub async fn sign_in_told(
    client: &ClientEntry,
    scopes: &[String],
    on_url: &dyn Fn(&str),
    now: UnixSeconds,
    wait: Duration,
) -> Result<Credential, SignInFault> {
    let endpoints = crate::clients::endpoints(client).map_err(SignInFault::Failed)?;
    let listener = LoopbackServer::bind_with(RedirectPath::Bare)
        .await
        .map_err(|e| SignInFault::Failed(RuntimeError::Connect(Failure::new("loopback", e))))?;
    let redirect = listener.redirect_uri();
    let pkce = Pkce::from_random(rand::random(), rand::random());
    on_url(&authorize_url(&endpoints, client, &pkce, &redirect, scopes));
    let code = listener
        .wait_until(&pkce.state, wait)
        .await
        .map_err(|fault| match fault {
            LoopbackFault::Refused(why) => SignInFault::Declined(why),
            LoopbackFault::TimedOut => SignInFault::TimedOut,
            other => SignInFault::Unfinished(other),
        })?;
    let http = crate::http::oauth_http().map_err(SignInFault::Failed)?;
    let tokens = exchange_code_scoped_detailed(
        &http,
        &endpoints,
        client,
        &pkce,
        &code,
        &redirect,
        redeem_scope(scopes).as_deref(),
    )
    .await
    .map_err(|fault| SignInFault::Failed(exchange_failure("token exchange failed", fault)))?;
    // No refresh token means the account works until the access token expires and then stops,
    // with no way to recover but a new browser round trip. Fail loudly now.
    let Some(refresh) = tokens.refresh_token.clone() else {
        return Err(SignInFault::Failed(RuntimeError::Secrets(Failure::said(
            "issuer returned no refresh token; access_type=offline may have been ignored",
        ))));
    };
    Ok(Credential::OAuth {
        expires_at: tokens.expires_at(now),
        access: tokens.access_token,
        refresh,
    })
}

/// Graph's scope for reading the signed-in person's own profile, which is where a Microsoft
/// account's address is.
pub const MICROSOFT_PROFILE_SCOPE: &str = "https://graph.microsoft.com/User.Read";

/// What a sign-in asks for beyond an account's own scopes, so that [`whoami`] can read whose
/// account it is: Google's `email` (its mail scopes already carry it) and Graph's profile for
/// Microsoft, whose mail token is for Exchange and says nothing of the person.
pub fn identity_scopes(issuer: Issuer) -> &'static [&'static str] {
    match issuer {
        Issuer::Google => &["email"],
        Issuer::Microsoft => &[MICROSOFT_PROFILE_SCOPE],
        _ => &[],
    }
}

/// Where [`whoami`] reads an account's address: OpenID Connect's userinfo at Google, Graph's
/// `/me` at Microsoft. A test points them at itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    pub google: String,
    pub microsoft: String,
}

impl Default for Profiles {
    fn default() -> Self {
        Profiles {
            google: "https://openidconnect.googleapis.com/v1/userinfo".to_owned(),
            microsoft: crate::graph::read::ME.to_owned(),
        }
    }
}

/// The address of the account `credential` was just signed in as, asked of its issuer.
///
/// Google answers with the sign-in's own access token. Microsoft's is for Exchange, so a token
/// for Graph's profile is minted from the refresh token first (the sign-in asked for it through
/// [`identity_scopes`]); Graph names the address as `mail`, or, for an account with no mailbox
/// address of its own, as its user principal name.
pub async fn whoami(
    client: &ClientEntry,
    credential: &Credential,
    at: &Profiles,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, RuntimeError> {
    let Credential::OAuth {
        access, refresh, ..
    } = credential
    else {
        return Err(RuntimeError::Secrets(Failure::said(
            "only a browser sign-in names its account",
        )));
    };
    let (url, token, fields): (&str, String, &[&str]) = match client.issuer {
        Issuer::Google => (
            at.google.as_str(),
            access.expose().to_owned(),
            &["email"][..],
        ),
        Issuer::Microsoft => {
            let minted = crate::tokens::refresh(
                client,
                refresh.expose(),
                &[MICROSOFT_PROFILE_SCOPE.to_owned()],
                now,
            )
            .await?;
            let Credential::OAuth { access, .. } = minted else {
                return Err(RuntimeError::Secrets(Failure::said(
                    "the issuer answered a refresh with no token",
                )));
            };
            (
                at.microsoft.as_str(),
                access.expose().to_owned(),
                &["mail", "userPrincipalName"][..],
            )
        }
        other => {
            return Err(RuntimeError::Secrets(Failure::said(format!(
                "no way to ask {other:?} whose account signed in"
            ))));
        }
    };
    let http = crate::http::http_client()?;
    let answer = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| RuntimeError::Connect(Failure::new("reading who signed in", e)))?;
    let status = answer.status();
    let body = answer
        .bytes()
        .await
        .map_err(|e| RuntimeError::Connect(Failure::new("reading who signed in", e)))?;
    if !status.is_success() {
        return Err(RuntimeError::Connect(Failure::said(format!(
            "reading who signed in: the issuer answered {status}"
        ))));
    }
    let profile: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| RuntimeError::Connect(Failure::new("reading who signed in", e)))?;
    fields
        .iter()
        .filter_map(|field| profile.get(field).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .find(|address| address.contains('@'))
        .map(str::to_lowercase)
        .ok_or_else(|| {
            RuntimeError::Connect(Failure::said(
                "reading who signed in: the issuer named no address",
            ))
        })
}
