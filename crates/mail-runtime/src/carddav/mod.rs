//! CardDAV (RFC 6352): finding a user's address books and reading them into the contacts table.
//!
//! The requests are built and the replies read by `mail_pim::dav`; this module sends them.
//! Cards come down and become contacts under [`mail_store::Origin::Book`], and a `KIND:group`
//! card becomes a [`mail_store::Group`]. The one thing written back is a group edited here
//! ([`mail_store::Edit::Edited`]): the next sync of its book sends it with `PUT`, conditional on
//! the etag last seen (RFC 6352 §6.3.2, RFC 9110 §13.1.1).
//!
//! HTTPS only, whatever URL is given: every request carries a password or a bearer token.
//! Redirects are followed by hand rather than by the client, because the one CardDAV depends on
//! — `/.well-known/carddav` (RFC 6764) — is usually a `301` answered to a `PROPFIND`, which the
//! client would turn into a `GET`; and because credentials go only to the origin they were given
//! for, never to wherever a redirect points.
//!
//! **Through accountd's relay** (step E7, [`Dav::relayed`]): an account that is the desktop's
//! accountd's has no password or token here. The requests go as plain HTTP/1.1, with no
//! `Authorization`, over the stream `Accounts::open_authenticated` returns for the account's
//! CardDAV endpoint, one stream for each request; the relay dials the server over TLS and adds the
//! credential (`relay.rs`). The https rule reads there as: the URL is the origin of the endpoint
//! the grant lists, and nothing else (the relay refuses any other origin, and a redirect to one
//! stops here before anything is written). The scheme is the endpoint's own, which porter allows
//! to be plain `http` only for a loopback host.

mod discover;
mod group;
mod relay;
mod sync;

pub use discover::{Collection, discover};
pub use group::{Unwritten, group_card};
pub use sync::{How, Synced, sync};

use crate::RuntimeError;
use crate::link::Accountd;
use mail_domain::{Retry, Retryable};
use porter_core::{GrantId, ServiceEndpoint};
use reqwest::{Method, StatusCode};
use std::fmt;
use std::sync::Arc;
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
    /// None at all: the desktop's accountd signs in on its relay, which adds the `Authorization`
    /// itself and drops any this side writes. Only [`Dav::relayed`] has it; no request of that
    /// session carries an `Authorization` header.
    Relayed,
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
            DavAuth::Relayed => f.write_str("DavAuth::Relayed"),
        }
    }
}

impl DavAuth {
    /// The `Authorization` value to send, if this side sends one.
    fn header(&self) -> Option<String> {
        use base64::Engine as _;
        match self {
            DavAuth::Basic { user, password } => Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
            )),
            DavAuth::Bearer(token) => Some(format!("Bearer {token}")),
            DavAuth::Relayed => None,
        }
    }
}

/// Why a CardDAV exchange did not work.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardDavFailure {
    /// `401`: the password or token was refused.
    #[error("the address book server refused the sign-in")]
    Unauthorized,
    /// Through accountd's relay: Mail is no longer allowed to read the account's contacts (the
    /// grant was withdrawn). Allowing Mail again answers it, not a sign-in.
    #[error("Mail is no longer allowed to read this account's contacts")]
    NotAllowed,
    /// Any other status the exchange could not go on from.
    #[error("the address book server answered {status} to {what}")]
    Refused { status: u16, what: &'static str },
    /// Discovery found no address book.
    #[error("no address book was found at {0}")]
    NotFound(String),
    /// A redirect or an href pointed somewhere other than `https:`.
    #[error("{0} is not an https address; nothing was sent there")]
    Insecure(String),
    /// Through accountd's relay: a redirect or an href named a server other than the account's
    /// CardDAV endpoint, which the relay would refuse. Nothing was sent there.
    #[error("{0} is not the account's address book server; nothing was sent there")]
    Foreign(String),
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
            CardDavFailure::NotAllowed => Retry::NeedsGrant,
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
    wire: Wire,
    auth: DavAuth,
    /// The origin the credentials were given for. Nothing else is sent them. Relayed, the origin
    /// of the endpoint the relay serves, and the only one a request may name.
    origin: url::Origin,
}

/// How the requests get to the server: a client of ours, or the stream accountd's relay hands over.
#[derive(Debug, Clone)]
enum Wire {
    Http(reqwest::Client),
    Relay(relay::Relay),
}

/// What came back from one request, before redirects and statuses are read.
#[derive(Debug)]
struct Raw {
    status: StatusCode,
    location: Option<String>,
    etag: Option<String>,
    body: String,
}

/// A reply: where it finally came from, its status, its `ETag` and its body.
#[derive(Debug)]
struct Reply {
    url: Url,
    status: StatusCode,
    etag: Option<String>,
    body: String,
}

/// What one request sends beyond its method and URL.
struct Request<'a> {
    depth: Option<&'a str>,
    content_type: &'static str,
    /// `If-Match`: the etag the resource must still have.
    if_match: Option<&'a str>,
    body: String,
}

/// How a `PUT` of a card went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Put {
    /// Stored. The new etag when the server said it; RFC 6352 §6.3.2.3 lets a server that
    /// changed what it stored leave it out.
    Stored(Option<String>),
    /// `412`: the card changed on the server since its etag was read.
    Changed,
}

impl Dav {
    /// A session with the server at `base`, which must be https.
    pub fn new(http: reqwest::Client, base: &Url, auth: DavAuth) -> Result<Self, RuntimeError> {
        if auth == DavAuth::Relayed {
            return Err(RuntimeError::Connect(
                "a session with no credential of its own goes through accountd's relay".to_owned(),
            ));
        }
        https(base)?;
        Ok(Self {
            wire: Wire::Http(http),
            auth,
            origin: base.origin(),
        })
    }

    /// A session with the account's CardDAV `endpoint`, through accountd's relay on `grant`
    /// (`Accounts::open_authenticated`): no credential here ([`DavAuth::Relayed`]), and no TLS
    /// either, the relay owns both. Every request opens its own stream, so a relay that is
    /// refused (`NeedsReauth`, a grant gone) says so on the request, and an idle connection is
    /// never found closed.
    pub fn relayed(
        link: Arc<dyn Accountd>,
        grant: GrantId,
        endpoint: ServiceEndpoint,
    ) -> Result<Self, RuntimeError> {
        let base = Url::parse(endpoint.url.as_str())
            .map_err(|e| RuntimeError::Connect(format!("{}: {e}", endpoint.url.as_str())))?;
        Ok(Self {
            origin: base.origin(),
            wire: Wire::Relay(relay::Relay::new(link, grant, endpoint)),
            auth: DavAuth::Relayed,
        })
    }

    /// The URL a session starts from when it is not given one: the endpoint the relay serves.
    /// `None` for a session of our own.
    pub fn endpoint_url(&self) -> Option<Url> {
        match &self.wire {
            Wire::Http(_) => None,
            Wire::Relay(relay) => Url::parse(relay.endpoint().url.as_str()).ok(),
        }
    }

    /// Whether a request may go to `url`: https, for a client of ours; the relay's endpoint
    /// origin, for the relay.
    fn secure(&self, url: &Url) -> Result<(), CardDavFailure> {
        match &self.wire {
            Wire::Http(_) => https(url),
            Wire::Relay(_) if url.origin() == self.origin => Ok(()),
            Wire::Relay(_) => Err(CardDavFailure::Foreign(url.to_string())),
        }
    }

    /// An href from a reply, as a URL: relative ones against the URL that was asked.
    fn resolve(&self, base: &Url, href: &str) -> Result<Url, CardDavFailure> {
        let url = base
            .join(href)
            .map_err(|e| CardDavFailure::Malformed(format!("{href}: {e}")))?;
        self.secure(&url)?;
        Ok(url)
    }

    /// Send one request, following redirects with the same method and body.
    async fn send(
        &self,
        method: &str,
        url: &Url,
        request: Request<'_>,
    ) -> Result<Reply, CardDavFailure> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|e| CardDavFailure::Malformed(e.to_string()))?;
        let mut at = url.clone();
        for _ in 0..=REDIRECTS {
            self.secure(&at)?;
            let mut headers: Vec<(&'static str, String)> =
                vec![("Content-Type", request.content_type.to_owned())];
            if let Some(depth) = request.depth {
                headers.push(("Depth", depth.to_owned()));
            }
            if let Some(etag) = request.if_match {
                headers.push(("If-Match", etag.to_owned()));
            }
            if at.origin() == self.origin
                && let Some(value) = self.auth.header()
            {
                headers.push(("Authorization", value));
            }
            let raw = match &self.wire {
                Wire::Http(http) => exchange(http, &method, &at, &headers, &request.body).await?,
                Wire::Relay(relay) => {
                    relay
                        .exchange(&method, &at, &headers, &request.body)
                        .await?
                }
            };
            if raw.status.is_redirection()
                && let Some(location) = &raw.location
            {
                at = at
                    .join(location)
                    .map_err(|e| CardDavFailure::Malformed(format!("{location}: {e}")))?;
                continue;
            }
            if raw.status == StatusCode::UNAUTHORIZED {
                return Err(CardDavFailure::Unauthorized);
            }
            return Ok(Reply {
                url: at,
                status: raw.status,
                etag: raw.etag,
                body: raw.body,
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
        let request = Request {
            depth: Some(depth),
            content_type: "application/xml; charset=utf-8",
            if_match: None,
            body,
        };
        let reply = self.send(method, url, request).await?;
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

    /// `PUT` the vCard `text` at `url` if the card there still has `etag` (RFC 6352 §6.3.2).
    /// An empty `etag` — a server that never gave one — sends it unconditionally.
    pub(crate) async fn put_card(
        &self,
        url: &Url,
        etag: &str,
        text: String,
    ) -> Result<Put, CardDavFailure> {
        let request = Request {
            depth: None,
            content_type: "text/vcard; charset=utf-8",
            if_match: (!etag.is_empty()).then_some(etag),
            body: text,
        };
        let reply = self.send("PUT", url, request).await?;
        match reply.status {
            status if status.is_success() => Ok(Put::Stored(reply.etag)),
            StatusCode::PRECONDITION_FAILED => Ok(Put::Changed),
            status => Err(CardDavFailure::Refused {
                status: status.as_u16(),
                what: "the PUT of a group",
            }),
        }
    }
}

fn https(url: &Url) -> Result<(), CardDavFailure> {
    if url.scheme() == "https" {
        Ok(())
    } else {
        Err(CardDavFailure::Insecure(url.to_string()))
    }
}

/// Whether two URLs name the same collection, a trailing slash either way.
fn same(a: &Url, b: &Url) -> bool {
    a.as_str().trim_end_matches('/') == b.as_str().trim_end_matches('/')
}

/// One request through a client of ours.
async fn exchange(
    http: &reqwest::Client,
    method: &Method,
    url: &Url,
    headers: &[(&'static str, String)],
    body: &str,
) -> Result<Raw, CardDavFailure> {
    let mut sending = http
        .request(method.clone(), url.clone())
        .body(body.to_owned());
    for (name, value) in headers {
        sending = sending.header(*name, value.as_str());
    }
    let response = sending.send().await.map_err(|e| unreachable(&e))?;
    let status = response.status();
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let (location, etag) = (
        header(reqwest::header::LOCATION),
        header(reqwest::header::ETAG),
    );
    let body = response.text().await.map_err(|e| unreachable(&e))?;
    Ok(Raw {
        status,
        location,
        etag,
        body,
    })
}

/// A failure of the transport, with the causes under it.
fn unreachable(err: &(dyn std::error::Error + 'static)) -> CardDavFailure {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    CardDavFailure::Unreachable(out)
}
