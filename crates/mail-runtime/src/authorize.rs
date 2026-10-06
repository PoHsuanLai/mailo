//! The browser sign-in: PKCE, a loopback redirect, and the exchange of the code.
//!
//! The rules (the authorize URL, what a redirect may be, the exchange) are porter-oauth's; this
//! is the part that needs a socket and randomness, which porter-oauth takes as arguments.

use crate::RuntimeError;
use porter_core::{Credential, UnixSeconds};
use porter_oauth::{
    ExchangeFailure, ExchangeFault, LoopbackFault, LoopbackServer, Pkce, RedirectPath,
    authorize_url, exchange_code_scoped_detailed, redeem_scope,
};
use porter_provider::ClientEntry;
use std::time::Duration;

/// Give up if no browser arrives. The user may have closed the tab or never signed in.
const WAIT: Duration = Duration::from_secs(300);

/// What a failed exchange with an issuer means. Only the issuer's own refusal is `Secrets`, which
/// reads as `NeedsReauth`: an issuer that cannot be reached this minute has refused nothing, and
/// calling it a rejected sign-in would stop a watch that only had to wait.
pub(crate) fn exchange_fault(what: &str, fault: ExchangeFault) -> RuntimeError {
    match fault {
        ExchangeFault::Refused => RuntimeError::Secrets(format!("{what}: the issuer refused it")),
        ExchangeFault::Unreachable => {
            RuntimeError::Connect(format!("{what}: token endpoint unreachable"))
        }
        ExchangeFault::Unreadable => {
            RuntimeError::Connect(format!("{what}: the token endpoint answered unreadably"))
        }
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
        && let RuntimeError::Secrets(text) | RuntimeError::Connect(text) = &mut error
    {
        text.push_str(": ");
        text.push_str(&said);
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
    let endpoints = crate::clients::endpoints(client)?;
    let listener = LoopbackServer::bind_with(RedirectPath::Bare)
        .await
        .map_err(|e| RuntimeError::Connect(format!("loopback: {e}")))?;
    let redirect = listener.redirect_uri();
    let pkce = Pkce::from_random(rand::random(), rand::random());
    on_url(&authorize_url(&endpoints, client, &pkce, &redirect, scopes));
    let code = listener
        .wait_until(&pkce.state, wait)
        .await
        .map_err(|fault| match fault {
            LoopbackFault::Refused(why) => {
                RuntimeError::Secrets(format!("authorization declined: {why}"))
            }
            other => RuntimeError::Secrets(format!("the sign-in did not complete: {other}")),
        })?;
    let http = crate::http::oauth_http()?;
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
    .map_err(|fault| exchange_failure("token exchange failed", fault))?;
    // No refresh token means the account works until the access token expires and then stops,
    // with no way to recover but a new browser round trip. Fail loudly now.
    let Some(refresh) = tokens.refresh_token.clone() else {
        return Err(RuntimeError::Secrets(
            "issuer returned no refresh token; access_type=offline may have been ignored"
                .to_owned(),
        ));
    };
    Ok(Credential::OAuth {
        expires_at: tokens.expires_at(now),
        access: tokens.access_token,
        refresh,
    })
}
