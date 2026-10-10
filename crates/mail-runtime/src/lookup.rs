//! The network half of looking things up: DNS and plain HTTPS fetches.
//!
//! Account discovery itself is `porter-discover`'s (what each answer means, and the order the
//! sources are asked in); it asks through two seams, `porter_discover::Dns` and
//! `porter_http::Http`, and this module implements both over what mailo already had: the
//! system resolver (hickory) and its `reqwest` client. No other resolver and no second HTTP
//! client enters the tree for it.
//!
//! [`SystemDns`] also answers BIMI's TXT lookups ([`crate::bimi::Txt`]), which is why it is a
//! type of its own and not part of a discovery module.
//!
//! HTTPS only, with certificate verification (reqwest's default, which nothing here turns off),
//! and no plain-HTTP fallback: a configuration document fetched in the clear could name any
//! server at all. Redirects are followed only to `https:`. Each request has [`PER_REQUEST`], and
//! a caller that wants a bound on a whole search wraps it in [`TOTAL`].

use crate::error::Failure;
use porter_discover::{Dns, DnsFault, MxRecord, SrvRecord};
use porter_http::{Header, Http, HttpError, HttpRequest, HttpResponse, Status};
use porter_provider::DomainName;
use std::fmt;
use std::time::Duration;

/// How long one HTTP request or DNS query may take.
pub const PER_REQUEST: Duration = Duration::from_secs(4);

/// How long a whole discovery search may take.
pub const TOTAL: Duration = Duration::from_secs(25);

/// The most a fetched document may weigh: a longer one is [`HttpError::TooLarge`]. Real
/// configuration documents are a few kilobytes.
pub const DOCUMENT_LIMIT: usize = 256 * 1024;

/// The most redirects to follow.
const REDIRECTS: usize = 3;

/// Why a DNS question was not answered with records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// The source answered: there is nothing there (no records).
    Absent,
    /// No answer: the resolver or the network failed, or it timed out.
    Unreachable(String),
}

/// The system's resolver, from `/etc/resolv.conf` or the platform's equivalent.
pub struct SystemDns(hickory_resolver::TokioResolver);

impl fmt::Debug for SystemDns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SystemDns")
    }
}

impl SystemDns {
    pub fn new() -> Result<Self, crate::RuntimeError> {
        let mut builder = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|e| crate::RuntimeError::Connect(Failure::new("no DNS configuration", e)))?;
        builder.options_mut().timeout = PER_REQUEST;
        builder.options_mut().attempts = 1;
        builder
            .build()
            .map(SystemDns)
            .map_err(|e| crate::RuntimeError::Connect(Failure::new("cannot start a resolver", e)))
    }

    /// The TXT records at `name`, each one's strings joined as RFC 7208 §3.3 joins SPF's. A
    /// string that is not UTF-8 is read lossily: every record read from TXT here is ASCII.
    pub(crate) async fn txt_records(&self, name: &str) -> Result<Vec<String>, Miss> {
        use hickory_resolver::proto::rr::RData;
        let lookup = self.0.txt_lookup(name).await.map_err(dns_miss)?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::TXT(txt) => Some(
                    txt.txt_data
                        .iter()
                        .map(|part| String::from_utf8_lossy(part))
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect())
    }
}

/// `porter-discover`'s DNS seam over the system resolver.
impl Dns for SystemDns {
    async fn srv(&self, name: &str) -> Result<Vec<SrvRecord>, DnsFault> {
        use hickory_resolver::proto::rr::RData;
        let lookup = self.0.srv_lookup(name).await.map_err(fault)?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                // A target that is not a name (`.`, the service decidedly not offered) is no host.
                RData::SRV(srv) => Some(SrvRecord {
                    priority: srv.priority,
                    weight: srv.weight,
                    port: srv.port,
                    target: DomainName::parse(&srv.target.to_utf8()).ok()?,
                }),
                _ => None,
            })
            .collect())
    }

    async fn mx(&self, domain: &DomainName) -> Result<Vec<MxRecord>, DnsFault> {
        use hickory_resolver::proto::rr::RData;
        let lookup = self.0.mx_lookup(domain.as_str()).await.map_err(fault)?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                // The null MX (RFC 7505) names `.`, which is no host and is left out.
                RData::MX(mx) => Some(MxRecord {
                    preference: mx.preference,
                    host: DomainName::parse(&mx.exchange.to_utf8()).ok()?,
                }),
                _ => None,
            })
            .collect())
    }
}

fn dns_miss(e: hickory_resolver::net::NetError) -> Miss {
    if e.is_no_records_found() {
        Miss::Absent
    } else {
        Miss::Unreachable(e.to_string())
    }
}

fn fault(e: hickory_resolver::net::NetError) -> DnsFault {
    match dns_miss(e) {
        Miss::Absent => DnsFault::NoRecords,
        Miss::Unreachable(_) => DnsFault::Unreachable,
    }
}

/// A client configured for fetching configuration documents: https only, [`PER_REQUEST`],
/// redirects only to `https:`.
///
/// A builder so a test can trust its own certificate and point names at a local listener;
/// everything that makes the fetch safe is already set.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(PER_REQUEST)
        .connect_timeout(PER_REQUEST)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= REDIRECTS || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
}

/// `porter_http`'s seam over a `reqwest` client.
///
/// Built from [`client_builder`], so it is https only, has [`PER_REQUEST`] and follows
/// redirects only to `https:`; a redirect it declines to follow is the 3xx response itself.
/// A body longer than [`DOCUMENT_LIMIT`] is [`HttpError::TooLarge`], read no further than that.
#[derive(Debug, Clone)]
pub struct ReqwestHttp(reqwest::Client);

impl ReqwestHttp {
    /// The client discovery uses.
    pub fn new() -> Result<Self, reqwest::Error> {
        client_builder().build().map(Self)
    }

    /// Over a client the caller built (a test, to trust its own certificate).
    pub fn over(client: reqwest::Client) -> Self {
        Self(client)
    }
}

impl Http for ReqwestHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let method = reqwest::Method::from_bytes(request.method.token().as_bytes())
            .map_err(|_| HttpError::Malformed)?;
        let mut pending = self.0.request(method, request.url.as_str());
        for header in &request.headers {
            pending = pending.header(header.name.as_str(), header.value.0.as_str());
        }
        if !request.body.is_empty() {
            pending = pending.body(request.body);
        }
        let mut response = pending.send().await.map_err(error)?;
        let status = Status(response.status().as_u16());
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some(Header::new(name.as_str(), value.to_str().ok()?)))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(error)? {
            if body.len() + chunk.len() > DOCUMENT_LIMIT {
                return Err(HttpError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// What a failed request is, for the seam: reqwest's error and its causes, which is where
/// "certificate not valid for name" lives.
fn error(e: reqwest::Error) -> HttpError {
    if e.is_timeout() {
        return HttpError::TimedOut;
    }
    if e.is_decode() || e.is_body() {
        return HttpError::Malformed;
    }
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    let text = text.to_ascii_lowercase();
    if ["certificate", "handshake", "tls"]
        .iter()
        .any(|word| text.contains(word))
    {
        HttpError::Tls
    } else {
        HttpError::Unreachable
    }
}
