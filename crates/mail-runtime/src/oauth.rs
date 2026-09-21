//! OAuth 2.0 with PKCE, for IMAP and SMTP `XOAUTH2`.
//!
//! This lives in the runtime rather than in `mail-proto`, which was a deliberate change. The
//! `oauth2` crate emits an `http::Request` rather than bytes, and `IoNeed` has no variant for
//! one; a token exchange is also stateless, so there is nothing to replay from a transcript and
//! the sans-I/O rule would buy nothing. Keeping it here also puts it beside the loopback
//! listener it needs.
//!
//! Gmail and Microsoft use the same wire format for the credential once it exists — see
//! [`xoauth2`] — so this module is shared.

use crate::RuntimeError;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::{Credential, OAuthIssuer};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
};
use std::time::Duration;

/// Refresh this long before a token actually expires.
///
/// A token that expires mid-`FETCH` fails the whole operation, and clock skew between us and
/// the issuer is real, so the margin is generous rather than tight.
const REFRESH_MARGIN: TimeDelta = match TimeDelta::try_minutes(5) {
    Some(d) => d,
    None => panic!("five minutes is a valid TimeDelta"),
};

/// Where an issuer's endpoints live.
struct Endpoints {
    auth: &'static str,
    token: &'static str,
}

fn endpoints(issuer: OAuthIssuer) -> Endpoints {
    match issuer {
        OAuthIssuer::Google => Endpoints {
            auth: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
        },
    }
}

/// Perform an `oauth2` HTTP request with our own client.
///
/// `oauth2` is built with default features off precisely so this is our call to make: the
/// runtime already owns TLS, timeouts and proxy behaviour, and a token exchange should not
/// quietly acquire a second HTTP stack with its own settings.
async fn send(
    http: &reqwest::Client,
    request: oauth2::HttpRequest,
) -> Result<oauth2::HttpResponse, RuntimeError> {
    let (parts, body) = request.into_parts();
    let response = http
        .request(
            reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
                .map_err(|e| RuntimeError::Secrets(format!("bad method: {e}")))?,
            parts.uri.to_string(),
        )
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|e| RuntimeError::Secrets(format!("token endpoint unreachable: {e}")))?;

    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| RuntimeError::Secrets(format!("token endpoint body: {e}")))?;

    let mut builder = oauth2::http::Response::builder().status(status.as_u16());
    for (name, value) in headers.iter() {
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    builder
        .body(bytes.to_vec())
        .map_err(|e| RuntimeError::Secrets(format!("bad token response: {e}")))
}

/// An authorization request in flight.
///
/// Holds the two secrets that make the redirect safe to accept, which is why it is not `Clone`
/// and not `Debug`: the verifier proves the code came back to the client that asked for it, and
/// the state proves the redirect is a reply to *our* request rather than one an attacker sent
/// the user's browser.
pub struct Pending {
    verifier: PkceCodeVerifier,
    state: CsrfToken,
    client_id: String,
    issuer: OAuthIssuer,
    redirect: String,
}

// Written by hand rather than derived, for the same reason `Credential` is: a derived Debug
// puts the PKCE verifier and the CSRF state into every log line, panic message and error chain
// that touches the value. Both are secrets — the verifier proves the code came back to the
// client that requested it, and the state is what makes a loopback redirect safe to accept.
impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("issuer", &self.issuer)
            .field("redirect", &self.redirect)
            .field("verifier", &"<redacted>")
            .field("state", &"<redacted>")
            .finish()
    }
}

/// The URL to open in a browser, and the state needed to finish.
#[derive(Debug)]
pub struct Authorization {
    pub url: String,
    pub pending: Pending,
}

/// Begin an authorization-code flow with PKCE.
///
/// `redirect` must be a loopback URL — `http://127.0.0.1:<port>` — because an installed
/// application cannot keep a client secret, and PKCE plus loopback is what replaces one.
pub fn begin(
    issuer: OAuthIssuer,
    client_id: &str,
    scopes: &[String],
    redirect: &str,
) -> Result<Authorization, RuntimeError> {
    let ends = endpoints(issuer);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let client = BasicClient::new(ClientId::new(client_id.to_owned()))
        .set_auth_uri(AuthUrl::new(ends.auth.to_owned()).map_err(bad_url)?)
        .set_token_uri(TokenUrl::new(ends.token.to_owned()).map_err(bad_url)?)
        .set_redirect_uri(RedirectUrl::new(redirect.to_owned()).map_err(bad_url)?);

    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge);
    for scope in scopes {
        request = request.add_scope(Scope::new(scope.clone()));
    }
    // Without this Google returns no refresh token on a repeat authorization, and the account
    // silently stops working an hour later.
    request = request.add_extra_param("access_type", "offline");
    request = request.add_extra_param("prompt", "consent");

    let (url, state) = request.url();
    Ok(Authorization {
        url: url.to_string(),
        pending: Pending {
            verifier,
            state,
            client_id: client_id.to_owned(),
            issuer,
            redirect: redirect.to_owned(),
        },
    })
}

impl Pending {
    /// Check that a redirect belongs to this request.
    ///
    /// Anything can reach a loopback listener, so a redirect whose `state` does not match is not
    /// a mistake to tolerate — it is someone else's authorization code being fed to us, which is
    /// exactly what the parameter exists to stop. Compared in constant time, since the value is
    /// a secret and a timing oracle on it is free to exploit.
    pub fn accepts(&self, state: &str) -> bool {
        let ours = self.state.secret().as_bytes();
        let theirs = state.as_bytes();
        if ours.len() != theirs.len() {
            return false;
        }
        ours.iter()
            .zip(theirs)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }

    /// Exchange an authorization code for a credential.
    pub async fn exchange(
        self,
        code: &str,
        http: &reqwest::Client,
        now: DateTime<Utc>,
    ) -> Result<Credential, RuntimeError> {
        let ends = endpoints(self.issuer);
        let client = BasicClient::new(ClientId::new(self.client_id))
            .set_auth_uri(AuthUrl::new(ends.auth.to_owned()).map_err(bad_url)?)
            .set_token_uri(TokenUrl::new(ends.token.to_owned()).map_err(bad_url)?)
            .set_redirect_uri(RedirectUrl::new(self.redirect).map_err(bad_url)?);

        let token = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(self.verifier)
            .request_async(&|req| send(http, req))
            .await
            .map_err(|e| RuntimeError::Secrets(format!("token exchange failed: {e}")))?;

        let refresh = token
            .refresh_token()
            .map(|r| r.secret().to_owned())
            // No refresh token means the account works until the access token expires and then
            // stops, with no way to recover but a new browser round trip. Fail loudly now.
            .ok_or_else(|| {
                RuntimeError::Secrets(
                    "issuer returned no refresh token; access_type=offline may have been ignored"
                        .to_owned(),
                )
            })?;

        Ok(Credential::OAuth {
            access: token.access_token().secret().to_owned(),
            refresh,
            expires_at: expiry(token.expires_in(), now),
        })
    }
}

/// Exchange a refresh token for a fresh access token.
///
/// The refresh token is carried through: issuers usually do not return a new one, and dropping
/// it would log the user out at the next expiry.
pub async fn refresh(
    issuer: OAuthIssuer,
    client_id: &str,
    refresh_token: &str,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    let ends = endpoints(issuer);
    let client = BasicClient::new(ClientId::new(client_id.to_owned()))
        .set_auth_uri(AuthUrl::new(ends.auth.to_owned()).map_err(bad_url)?)
        .set_token_uri(TokenUrl::new(ends.token.to_owned()).map_err(bad_url)?);

    let token = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_owned()))
        .request_async(&|req| send(http, req))
        .await
        .map_err(|e| RuntimeError::Secrets(format!("refresh failed: {e}")))?;

    Ok(Credential::OAuth {
        access: token.access_token().secret().to_owned(),
        refresh: token
            .refresh_token()
            .map(|r| r.secret().to_owned())
            .unwrap_or_else(|| refresh_token.to_owned()),
        expires_at: expiry(token.expires_in(), now),
    })
}

fn expiry(lifetime: Option<Duration>, now: DateTime<Utc>) -> DateTime<Utc> {
    let seconds = lifetime.map_or(3600, |d| d.as_secs().min(i64::MAX as u64) as i64);
    now + TimeDelta::try_seconds(seconds).unwrap_or(TimeDelta::zero())
}

/// Whether a credential should be refreshed before it is used.
pub fn needs_refresh(credential: &Credential, now: DateTime<Utc>) -> bool {
    match credential {
        Credential::Password(_) => false,
        Credential::OAuth { expires_at, .. } => *expires_at - REFRESH_MARGIN <= now,
    }
}

/// The SASL `XOAUTH2` initial response.
///
/// `base64("user=" + address + "\x01auth=Bearer " + token + "\x01\x01")`. Identical for Gmail and
/// for Exchange Online, which is why adding Microsoft is a preset row rather than a new backend.
pub fn xoauth2(address: &str, access_token: &str) -> String {
    STANDARD.encode(format!(
        "user={address}\x01auth=Bearer {access_token}\x01\x01"
    ))
}

fn bad_url(e: oauth2::url::ParseError) -> RuntimeError {
    RuntimeError::Secrets(format!("bad endpoint url: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &str = "test-client.apps.googleusercontent.com";
    const REDIRECT: &str = "http://127.0.0.1:8080";

    fn scopes() -> Vec<String> {
        vec!["https://mail.google.com/".to_owned(), "email".to_owned()]
    }

    #[test]
    fn the_authorize_url_carries_pkce_state_and_offline_access() {
        let auth = begin(OAuthIssuer::Google, CLIENT, &scopes(), REDIRECT).unwrap();
        for required in [
            "code_challenge=",
            "code_challenge_method=S256",
            "state=",
            // Without offline access Google returns no refresh token on a repeat
            // authorization and the account stops working an hour later.
            "access_type=offline",
            "response_type=code",
        ] {
            assert!(
                auth.url.contains(required),
                "missing {required} in {}",
                auth.url
            );
        }
        assert!(
            !auth.url.contains("client_secret"),
            "an installed app has none"
        );
    }

    #[test]
    fn a_redirect_with_the_wrong_state_is_refused() {
        // Anything can reach a loopback listener. A mismatched state is someone else's
        // authorization code being offered to us, which is the whole reason for the parameter.
        let auth = begin(OAuthIssuer::Google, CLIENT, &scopes(), REDIRECT).unwrap();
        for wrong in ["", "nonsense", "0000000000000000000000"] {
            assert!(!auth.pending.accepts(wrong), "accepted {wrong:?}");
        }
    }

    #[test]
    fn a_redirect_with_the_matching_state_is_accepted() {
        let auth = begin(OAuthIssuer::Google, CLIENT, &scopes(), REDIRECT).unwrap();
        let ours = auth.pending.state.secret().clone();
        assert!(auth.pending.accepts(&ours));
    }

    #[test]
    fn two_authorizations_never_share_a_verifier_or_state() {
        let a = begin(OAuthIssuer::Google, CLIENT, &scopes(), REDIRECT).unwrap();
        let b = begin(OAuthIssuer::Google, CLIENT, &scopes(), REDIRECT).unwrap();
        assert_ne!(a.pending.state.secret(), b.pending.state.secret());
        assert_ne!(a.pending.verifier.secret(), b.pending.verifier.secret());
    }

    #[test]
    fn xoauth2_matches_the_documented_encoding() {
        // Google and Microsoft document the identical format, so this string is the reason
        // adding Microsoft is a preset row rather than a second backend.
        let encoded = xoauth2("ada@example.test", "ya29.token");
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "user=ada@example.test\x01auth=Bearer ya29.token\x01\x01"
        );
    }

    #[test]
    fn refresh_is_due_before_expiry_not_after() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let fresh = Credential::OAuth {
            access: "a".into(),
            refresh: "r".into(),
            expires_at: now + TimeDelta::try_minutes(30).unwrap(),
        };
        assert!(!needs_refresh(&fresh, now));
        // Inside the margin: a token expiring mid-FETCH fails the whole operation.
        let soon = Credential::OAuth {
            access: "a".into(),
            refresh: "r".into(),
            expires_at: now + TimeDelta::try_minutes(2).unwrap(),
        };
        assert!(needs_refresh(&soon, now));
        assert!(!needs_refresh(&Credential::Password("p".into()), now));
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    /// The Debug impls must not leak what they hold.
    ///
    /// Both values are secrets and both end up inside error chains during setup, which is
    /// exactly when a developer is most likely to be printing things.
    #[test]
    fn debug_never_prints_the_verifier_or_the_state() {
        let auth = begin(
            OAuthIssuer::Google,
            "client.apps.googleusercontent.com",
            &["https://mail.google.com/".to_owned()],
            "http://127.0.0.1:8080",
        )
        .unwrap();
        let verifier = auth.pending.verifier.secret().clone();
        let state = auth.pending.state.secret().clone();
        let rendered = format!("{:?}", auth.pending);
        assert!(!rendered.contains(&verifier), "verifier leaked: {rendered}");
        assert!(!rendered.contains(&state), "state leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
    }
}
