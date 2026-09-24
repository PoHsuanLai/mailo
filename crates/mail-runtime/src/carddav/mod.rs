//! CardDAV (RFC 6352): finding a user's address books and reading them into the contacts table.
//!
//! The requests are built and the replies read by `mail_pim::dav`; this module sends them.
//! Read-only: cards come down and become contacts under [`mail_store::Origin::Book`], and
//! nothing is written back to the server.
//!
//! HTTPS only, whatever URL is given: every request carries a password or a bearer token.
//! Redirects are followed by hand rather than by the client, because the one CardDAV depends on
//! — `/.well-known/carddav` (RFC 6764) — is usually a `301` answered to a `PROPFIND`, which the
//! client would turn into a `GET`; and because credentials go only to the origin they were given
//! for, never to wherever a redirect points.

mod discover;
mod sync;

pub use discover::{Collection, discover};
pub use sync::{How, Synced, sync};

use crate::RuntimeError;
use mail_domain::{Retry, Retryable};
use reqwest::{Method, StatusCode};
use std::fmt;
use std::time::Duration;
use url::Url;

/// How long one request may take. Address books are small, but a first multiget of a large one
/// is a few megabytes over a phone connection.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// The most redirects one request follows.
const REDIRECTS: usize = 5;

/// How a CardDAV server is signed in to.
#[derive(Clone, PartialEq, Eq)]
pub enum DavAuth {
    /// HTTP Basic (RFC 7617), for a server with its own password.
    Basic { user: String, password: String },
    /// An OAuth bearer token (RFC 6750): the account's own sign-in, for a provider whose
    /// address book accepts it.
    Bearer(String),
}

// By hand, like `Credential`'s: a derived Debug puts the secret in every log line.
impl fmt::Debug for DavAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DavAuth::Basic { user, .. } => f
                .debug_struct("DavAuth::Basic")
                .field("user", user)
                .field("password", &"<redacted>")
                .finish(),
            DavAuth::Bearer(_) => f.write_str("DavAuth::Bearer(<redacted>)"),
        }
    }
}

impl DavAuth {
    fn header(&self) -> String {
        use base64::Engine as _;
        match self {
            DavAuth::Basic { user, password } => format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
            ),
            DavAuth::Bearer(token) => format!("Bearer {token}"),
        }
    }
}

/// Why a CardDAV exchange did not work.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardDavFailure {
    /// `401`: the password or token was refused.
    #[error("the address book server refused the sign-in")]
    Unauthorized,
    /// Any other status the exchange could not go on from.
    #[error("the address book server answered {status} to {what}")]
    Refused { status: u16, what: &'static str },
    /// Discovery found no address book.
    #[error("no address book was found at {0}")]
    NotFound(String),
    /// A redirect or an href pointed somewhere other than `https:`.
    #[error("{0} is not an https address; nothing was sent there")]
    Insecure(String),
    /// A redirect loop, or a chain longer than [`REDIRECTS`].
    #[error("too many redirects from {0}")]
    Redirects(String),
    /// The reply was not the document asked for.
    #[error("{0}")]
    Malformed(String),
    /// No answer: the name did not resolve, the connection failed, TLS failed, or it timed out.
    #[error("could not reach the address book server: {0}")]
    Unreachable(String),
}

impl Retryable for CardDavFailure {
    fn retry(&self) -> Retry {
        match self {
            CardDavFailure::Unauthorized => Retry::NeedsReauth,
            CardDavFailure::Refused {
                status: 429 | 500..=599,
                ..
            } => Retry::After(Duration::from_secs(60)),
            CardDavFailure::Unreachable(_) => Retry::After(Duration::from_secs(5)),
            other => Retry::Fatal(other.to_string()),
        }
    }
}

impl From<CardDavFailure> for RuntimeError {
    fn from(failure: CardDavFailure) -> Self {
        RuntimeError::CardDav(failure)
    }
}

/// A client for CardDAV: https only, [`TIMEOUT`], redirects left to [`Dav`].
///
/// A builder so a test can add a root it trusts and point a name at a local listener.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(15))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
}

/// [`client_builder`], built.
pub fn client() -> Result<reqwest::Client, RuntimeError> {
    client_builder()
        .build()
        .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))
}

/// One server, signed in to.
#[derive(Debug, Clone)]
pub struct Dav {
    http: reqwest::Client,
    auth: DavAuth,
    /// The origin the credentials were given for. Nothing else is sent them.
    origin: url::Origin,
}

/// A reply: where it finally came from, its status and its body.
#[derive(Debug)]
struct Reply {
    url: Url,
    status: StatusCode,
    body: String,
}

impl Dav {
    /// A session with the server at `base`, which must be https.
    pub fn new(http: reqwest::Client, base: &Url, auth: DavAuth) -> Result<Self, RuntimeError> {
        https(base)?;
        Ok(Self {
            http,
            auth,
            origin: base.origin(),
        })
    }

    /// Send one request, following redirects with the same method and body.
    async fn send(
        &self,
        method: &str,
        url: &Url,
        depth: Option<&str>,
        body: String,
    ) -> Result<Reply, CardDavFailure> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|e| CardDavFailure::Malformed(e.to_string()))?;
        let mut at = url.clone();
        for _ in 0..=REDIRECTS {
            https(&at)?;
            let mut request = self
                .http
                .request(method.clone(), at.clone())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/xml; charset=utf-8",
                )
                .body(body.clone());
            if let Some(depth) = depth {
                request = request.header("Depth", depth);
            }
            if at.origin() == self.origin {
                request = request.header(reqwest::header::AUTHORIZATION, self.auth.header());
            }
            let response = request.send().await.map_err(unreachable)?;
            let status = response.status();
            if status.is_redirection()
                && let Some(location) = response.headers().get(reqwest::header::LOCATION)
            {
                let location = location
                    .to_str()
                    .map_err(|e| CardDavFailure::Malformed(e.to_string()))?;
                at = at
                    .join(location)
                    .map_err(|e| CardDavFailure::Malformed(format!("{location}: {e}")))?;
                continue;
            }
            if status == StatusCode::UNAUTHORIZED {
                return Err(CardDavFailure::Unauthorized);
            }
            let body = response.text().await.map_err(unreachable)?;
            return Ok(Reply {
                url: at,
                status,
                body,
            });
        }
        Err(CardDavFailure::Redirects(url.to_string()))
    }

    /// A `PROPFIND` or `REPORT` whose answer must be `207 Multi-Status`.
    async fn multistatus(
        &self,
        method: &str,
        url: &Url,
        depth: &str,
        body: String,
        what: &'static str,
    ) -> Result<(Url, mail_pim::Multistatus), CardDavFailure> {
        let reply = self.send(method, url, Some(depth), body).await?;
        if reply.status != StatusCode::MULTI_STATUS {
            return Err(CardDavFailure::Refused {
                status: reply.status.as_u16(),
                what,
            });
        }
        let parsed = mail_pim::dav::multistatus(&reply.body)
            .map_err(|e| CardDavFailure::Malformed(e.to_string()))?;
        Ok((reply.url, parsed))
    }
}

fn https(url: &Url) -> Result<(), CardDavFailure> {
    if url.scheme() == "https" {
        Ok(())
    } else {
        Err(CardDavFailure::Insecure(url.to_string()))
    }
}

/// An href from a reply, as a URL: relative ones against the URL that was asked.
fn resolve(base: &Url, href: &str) -> Result<Url, CardDavFailure> {
    let url = base
        .join(href)
        .map_err(|e| CardDavFailure::Malformed(format!("{href}: {e}")))?;
    https(&url)?;
    Ok(url)
}

/// Whether two URLs name the same collection, a trailing slash either way.
fn same(a: &Url, b: &Url) -> bool {
    a.as_str().trim_end_matches('/') == b.as_str().trim_end_matches('/')
}

fn unreachable(err: reqwest::Error) -> CardDavFailure {
    let mut out = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    CardDavFailure::Unreachable(out)
}
