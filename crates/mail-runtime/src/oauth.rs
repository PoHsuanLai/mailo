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
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, Scope, TokenResponse, TokenUrl,
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
///
/// Public and substitutable, which is not only a test seam. Microsoft operates sovereign clouds
/// on different hosts — `login.microsoftonline.us` for US Government tenants — and a deployment
/// behind an inspecting proxy may need to point somewhere else again. An issuer is a value here
/// for the same reason a provider is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
}

/// The published endpoints for an issuer.
pub fn endpoints_for(issuer: OAuthIssuer) -> Endpoints {
    let e = endpoints(issuer);
    Endpoints {
        auth: e.auth.to_owned(),
        token: e.token.to_owned(),
    }
}

struct Wellknown {
    auth: &'static str,
    token: &'static str,
}

fn endpoints(issuer: OAuthIssuer) -> Wellknown {
    match issuer {
        OAuthIssuer::Google => Wellknown {
            auth: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
        },
        // `organizations`, not `common`. `common` also accepts personal Outlook.com accounts,
        // and those are a worse case that this does not support: basic authentication was
        // retired there on 2024-09-16 and recently-created personal mailboxes are reported to
        // have SMTP client authentication permanently off, so a `common` endpoint would hand
        // back a perfectly good token that then fails at submission with nothing to explain it.
        // Refusing at sign-in, where the user can read the reason, is the better failure.
        OAuthIssuer::Microsoft => Wellknown {
            auth: "https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize",
            token: "https://login.microsoftonline.com/organizations/oauth2/v2.0/token",
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
        // `Connect`, not `Secrets`: an issuer that cannot be reached this minute has not refused
        // anything, and calling it a rejected sign-in would stop a watch that only had to wait.
        .map_err(|e| RuntimeError::Connect(format!("token endpoint unreachable: {e}")))?;

    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| RuntimeError::Connect(format!("token endpoint body: {e}")))?;

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
    /// Google issues one even for a "Desktop app" client and its token endpoint **requires**
    /// it, PKCE or no PKCE — a sign-in without it fails with `client_secret is missing`.
    /// Microsoft's public clients genuinely have none, hence the `Option`.
    ///
    /// It is not a secret in the sense the name suggests: an installed application ships it to
    /// every user, Google documents it as not confidential for this client type, and what
    /// actually protects the exchange is PKCE plus the loopback redirect. Treated with care
    /// anyway — never logged, never in a URL.
    client_secret: Option<String>,
    issuer: OAuthIssuer,
    redirect: String,
    /// Resolved when the request was made, not looked up again when it completes: a second
    /// lookup is a second answer, and the code came back from whichever host was asked.
    endpoints: Endpoints,
    /// The scopes to name when redeeming the code, or none to name none. See [`first_resource`].
    redeem: Option<String>,
}

impl Pending {
    /// Send the exchange somewhere other than the issuer's published host.
    ///
    /// For a sovereign cloud, a proxy, or a test that must not reach the internet.
    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }
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
/// `redirect` must be a loopback URL — `http://127.0.0.1:<port>`. PKCE plus loopback is what
/// makes an installed application safe without a *confidential* secret.
///
/// `client_secret` is nonetheless required by Google, which issues one with every "Desktop app"
/// client and refuses the token exchange without it (`invalid_request: client_secret is
/// missing`). This code asserted the opposite for a long time, and the assertion held right up
/// until a real Google client was used. Microsoft's public clients want none, so it is optional.
pub fn begin(
    issuer: OAuthIssuer,
    client_id: &str,
    client_secret: Option<&str>,
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
            client_secret: client_secret.map(str::to_owned),
            issuer,
            redirect: redirect.to_owned(),
            endpoints: endpoints_for(issuer),
            redeem: first_resource(scopes),
        },
    })
}

/// When `scopes` span more than one resource, the ones the code should be redeemed for.
///
/// Microsoft lets one sign-in consent to several resources — Exchange's IMAP and Graph's
/// `Mail.Send` — but issues each access token for one, and refuses to redeem such a code without
/// being told which (`AADSTS28003`). The first resource named is the one the sign-in is for;
/// the others are reached later by refreshing with their scopes. Scopes that belong to no
/// resource (`offline_access`, `openid`) go with it. A request for one resource names nothing
/// here, and is redeemed exactly as before.
fn first_resource(scopes: &[String]) -> Option<String> {
    let resource = |s: &str| {
        s.strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .map(str::to_owned)
    };
    let resources: Vec<String> = scopes.iter().filter_map(|s| resource(s)).collect();
    let first = resources.first()?;
    if resources.iter().all(|r| r == first) {
        return None;
    }
    let chosen: Vec<&str> = scopes
        .iter()
        .filter(|s| resource(s).is_none_or(|r| &r == first))
        .map(String::as_str)
        .collect();
    Some(chosen.join(" "))
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
        let ends = self.endpoints;
        let mut client = BasicClient::new(ClientId::new(self.client_id))
            .set_auth_uri(AuthUrl::new(ends.auth.clone()).map_err(bad_url)?)
            .set_token_uri(TokenUrl::new(ends.token.clone()).map_err(bad_url)?)
            .set_redirect_uri(RedirectUrl::new(self.redirect).map_err(bad_url)?);
        if let Some(secret) = self.client_secret {
            client = client.set_client_secret(ClientSecret::new(secret));
        }

        let mut request = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(self.verifier);
        if let Some(scope) = &self.redeem {
            request = request.add_extra_param("scope", scope.clone());
        }
        let token = request
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
    client_secret: Option<&str>,
    refresh_token: &str,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    refresh_at(
        &endpoints_for(issuer),
        client_id,
        client_secret,
        refresh_token,
        http,
        now,
    )
    .await
}

/// The same, against endpoints the caller names.
pub async fn refresh_at(
    ends: &Endpoints,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    refresh_for(
        ends,
        client_id,
        client_secret,
        refresh_token,
        &[],
        http,
        now,
    )
    .await
}

/// A refresh that names the scopes the new access token is for.
///
/// Microsoft issues one access token per resource. A sign-in consented to Exchange's IMAP and
/// to Graph's `Mail.Send` returns a token for the first; a token for the second is this call,
/// with Graph's scope. No scopes asks for what the sign-in was for, as [`refresh_at`] does.
pub async fn refresh_for(
    ends: &Endpoints,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    scopes: &[String],
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<Credential, RuntimeError> {
    // The renewal needs the secret for the same reason the first exchange did. Without it an
    // account signs in, works for an hour, and then cannot be renewed — which is the failure
    // `signin::renew` exists to prevent, arriving by a different door.
    let mut client = BasicClient::new(ClientId::new(client_id.to_owned()))
        .set_auth_uri(AuthUrl::new(ends.auth.clone()).map_err(bad_url)?)
        .set_token_uri(TokenUrl::new(ends.token.clone()).map_err(bad_url)?);
    if let Some(secret) = client_secret {
        client = client.set_client_secret(ClientSecret::new(secret.to_owned()));
    }

    let token = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_owned()))
        .add_scopes(scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(&|req| send(http, req))
        .await
        .map_err(refresh_failed)?;

    Ok(Credential::OAuth {
        access: token.access_token().secret().to_owned(),
        refresh: token
            .refresh_token()
            .map(|r| r.secret().to_owned())
            .unwrap_or_else(|| refresh_token.to_owned()),
        expires_at: expiry(token.expires_in(), now),
    })
}

/// What a failed refresh means, which is not the same thing each time.
///
/// Only an answer from the issuer is a refusal: `invalid_grant` is a revoked or expired grant,
/// and no amount of retrying brings it back, so it is `Secrets` and reads as `NeedsReauth`. A
/// token endpoint that could not be reached, or answered with something that is not OAuth (a
/// proxy's error page, a 503), has refused nothing and is worth trying again later. Treating the
/// second like the first would end a long-running watch over a network blip.
fn refresh_failed<T: oauth2::ErrorResponse + 'static>(
    e: oauth2::RequestTokenError<RuntimeError, T>,
) -> RuntimeError {
    match e {
        oauth2::RequestTokenError::ServerResponse(_) => {
            RuntimeError::Secrets(format!("the issuer refused to renew the sign-in: {e}"))
        }
        oauth2::RequestTokenError::Request(inner) => inner,
        oauth2::RequestTokenError::Parse(..) | oauth2::RequestTokenError::Other(_) => {
            RuntimeError::Connect(format!("the token endpoint answered unreadably: {e}"))
        }
    }
}

/// The scopes to name when renewing the access token the incoming server signs in with.
///
/// Empty when the sign-in was for one resource, which asks for what it was for — Google's
/// `mail.google.com`, or Exchange's IMAP and SMTP together. A sign-in that also consented to a
/// second resource (Graph's `Mail.Send`) must name the first one's scopes on every refresh:
/// Microsoft issues each access token for one resource and refuses a request that spans two
/// (`AADSTS28003`).
pub fn incoming_scopes(scopes: &[String]) -> Vec<String> {
    first_resource(scopes)
        .map(|chosen| chosen.split(' ').map(str::to_owned).collect())
        .unwrap_or_default()
}

fn expiry(lifetime: Option<Duration>, now: DateTime<Utc>) -> DateTime<Utc> {
    let seconds = lifetime.map_or(3600, |d| d.as_secs().min(i64::MAX as u64) as i64);
    now + TimeDelta::try_seconds(seconds).unwrap_or(TimeDelta::zero())
}

/// What a credential needs before it can be used.
///
/// A value rather than a `bool`, and returned rather than acted on: deciding is a pure function
/// of the credential and the clock, and it carries the refresh token the decision implies so the
/// caller cannot reach the refreshing branch without one in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness<'a> {
    /// Usable as it stands. A password is always this; it does not expire on a schedule.
    Ready,
    /// Expired, or close enough that it would expire in the middle of an operation.
    Expired { refresh_token: &'a str },
}

/// Whether a credential can be used as it stands.
pub fn assess(credential: &Credential, now: DateTime<Utc>) -> Freshness<'_> {
    match credential {
        // Neither expires on a schedule.
        Credential::Password(_) | Credential::OpenPgp(_) | Credential::SmimeKey(_) => {
            Freshness::Ready
        }
        Credential::OAuth {
            refresh,
            expires_at,
            ..
        } if *expires_at - REFRESH_MARGIN <= now => Freshness::Expired {
            refresh_token: refresh,
        },
        Credential::OAuth { .. } => Freshness::Ready,
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
        let auth = begin(OAuthIssuer::Google, CLIENT, None, &scopes(), REDIRECT).unwrap();
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
        let auth = begin(OAuthIssuer::Google, CLIENT, None, &scopes(), REDIRECT).unwrap();
        for wrong in ["", "nonsense", "0000000000000000000000"] {
            assert!(!auth.pending.accepts(wrong), "accepted {wrong:?}");
        }
    }

    #[test]
    fn a_redirect_with_the_matching_state_is_accepted() {
        let auth = begin(OAuthIssuer::Google, CLIENT, None, &scopes(), REDIRECT).unwrap();
        let ours = auth.pending.state.secret().clone();
        assert!(auth.pending.accepts(&ours));
    }

    #[test]
    fn two_authorizations_never_share_a_verifier_or_state() {
        let a = begin(OAuthIssuer::Google, CLIENT, None, &scopes(), REDIRECT).unwrap();
        let b = begin(OAuthIssuer::Google, CLIENT, None, &scopes(), REDIRECT).unwrap();
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
    fn a_code_for_two_resources_is_redeemed_for_the_first() {
        let scopes = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            first_resource(&scopes(&[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "offline_access",
                "openid",
                "https://graph.microsoft.com/Mail.Send",
            ])),
            Some("https://outlook.office.com/IMAP.AccessAsUser.All offline_access openid".into())
        );
        // One resource: nothing is named, and the exchange is what it always was.
        assert_eq!(
            first_resource(&scopes(&[
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
                "offline_access",
            ])),
            None
        );
        assert_eq!(
            first_resource(&scopes(&["https://mail.google.com/", "email"])),
            None
        );
    }

    #[test]
    fn refresh_is_due_before_expiry_not_after() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let oauth = |minutes: i64| Credential::OAuth {
            access: "a".into(),
            refresh: "the-refresh-token".into(),
            expires_at: now + TimeDelta::try_minutes(minutes).unwrap(),
        };
        assert_eq!(assess(&oauth(30), now), Freshness::Ready);
        // Inside the margin: a token expiring mid-FETCH fails the whole operation.
        assert_eq!(
            assess(&oauth(2), now),
            Freshness::Expired {
                refresh_token: "the-refresh-token"
            }
        );
        // And past it, which is the state every OAuth account reached an hour after setup.
        assert_eq!(
            assess(&oauth(-120), now),
            Freshness::Expired {
                refresh_token: "the-refresh-token"
            }
        );
        assert_eq!(
            assess(&Credential::Password("p".into()), now),
            Freshness::Ready
        );
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
            None,
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
