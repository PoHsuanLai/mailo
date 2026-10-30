//! The network half of looking things up: DNS and plain HTTPS fetches.
//!
//! Account discovery itself is `porter-discover`'s (what each answer means, and the order the
//! sources are asked in); it asks through two seams, `porter_discover::Dns` and
//! `porter_http::Http`. The first this module implements over the system resolver (hickory);
//! the second is porter's own client (`porter_http_reqwest`), set to this module's limits.
//!
//! [`SystemDns`] also answers BIMI's TXT lookups (`mail_runtime::bimi::Txt`), which is why it is a
//! type of its own and not part of a discovery module.
//!
//! HTTPS only, with certificate verification, and no plain-HTTP fallback: a configuration
//! document fetched in the clear could name any server at all (porter's client dials `http` only
//! to this computer). Porter's client follows no redirect; `porter-discover` reads a `Location`
//! where it means to follow one, and discovery's fetches go through [`follow`]. [`client_builder`]'s
//! clients follow them to `https:` only.
//! Each request has [`PER_REQUEST`], and a caller that wants a bound on a whole search wraps it
//! in [`TOTAL`].

use crate::error::Failure;
use porter_discover::{Dns, DnsFault, MxRecord, SrvRecord};
use porter_http::http::{Method, Request, Response, Uri, header};
use porter_http::{HeaderText, Http, HttpError};
use porter_http_reqwest::{Limits, ReqwestHttp};
use porter_provider::DomainName;
use std::fmt;
use std::time::Duration;

/// How long one HTTP request or DNS query may take.
pub const PER_REQUEST: Duration = Duration::from_secs(4);

/// How long a whole discovery search may take.
pub const TOTAL: Duration = Duration::from_secs(25);

/// The most a fetched document may weigh: a longer one is [`porter_http::HttpError::TooLarge`]. Real
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
    pub fn new() -> Result<Self, crate::NetError> {
        let mut builder = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|e| crate::NetError::Connect(Failure::new("no DNS configuration", e)))?;
        builder.options_mut().timeout = PER_REQUEST;
        builder.options_mut().attempts = 1;
        builder
            .build()
            .map(SystemDns)
            .map_err(|e| crate::NetError::Connect(Failure::new("cannot start a resolver", e)))
    }

    /// The TXT records at `name`, each one's strings joined as RFC 7208 §3.3 joins SPF's. A
    /// string that is not UTF-8 is read lossily: every record read from TXT here is ASCII.
    pub async fn txt_records(&self, name: &str) -> Result<Vec<String>, Miss> {
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
/// Not kept to the public internet as [`crate::public`]'s clients are: discovery starts from the
/// person's own address, and a self-hosted or split-horizon server on their network (a private
/// address, a `.home.arpa` name) is the one they are adding.
///
/// A builder so a test can trust its own certificate and point names at a local listener;
/// everything that makes the fetch safe is already set.
pub fn client_builder() -> reqwest::ClientBuilder {
    crate::roots::trusting_system(reqwest::Client::builder())
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

/// The client discovery fetches through: porter's, with [`PER_REQUEST`] for a whole exchange and
/// [`DOCUMENT_LIMIT`] bytes of body (a longer one is [`porter_http::HttpError::TooLarge`]), and
/// redirects followed by [`follow`].
pub fn http() -> Following<ReqwestHttp> {
    Following(ReqwestHttp::new().with_limits(Limits {
        timeout: PER_REQUEST,
        max_body: DOCUMENT_LIMIT,
    }))
}

/// An [`Http`] that sends every request through [`follow`].
#[derive(Debug, Clone)]
pub struct Following<H>(H);

impl<H: Http> Http for Following<H> {
    async fn send(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, HttpError> {
        follow(&self.0, request).await
    }
}

/// Sends `request` and follows its redirects, which porter's client does not.
///
/// The one rule every discovery fetch goes by: at most [`REDIRECTS`] hops, and only to `https:`.
/// Discovery starts from the person's own address, so a redirect to a server on their own network
/// (a private address, a `.home.arpa` name) is followed like any other: a self-hosted server is
/// the one they are adding. A redirect that is not followed is the answer, for the caller to
/// read: one to plain `http:`, one with no usable `Location`, and the one past the last hop. A hop
/// to another origin drops the request's `Authorization`, `Cookie` and `Proxy-Authorization`.
pub async fn follow<H: Http>(
    http: &H,
    request: Request<Vec<u8>>,
) -> Result<Response<Vec<u8>>, HttpError> {
    let mut request = request;
    let mut hops = 0;
    loop {
        let response = http.send(request.clone()).await?;
        if hops == REDIRECTS || !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(next) = response
            .header("location")
            .and_then(|location| redirect_uri(request.uri(), location))
        else {
            return Ok(response);
        };
        // Credentials are for the server they were written for: another origin gets none.
        if next.scheme() != request.uri().scheme() || next.authority() != request.uri().authority()
        {
            for name in [
                header::AUTHORIZATION,
                header::COOKIE,
                header::PROXY_AUTHORIZATION,
            ] {
                request.headers_mut().remove(name);
            }
        }
        *request.uri_mut() = next;
        // The next request is a plain read of the new place, whatever the first one carried.
        *request.method_mut() = Method::GET;
        *request.body_mut() = Vec::new();
        hops += 1;
    }
}

/// Where `location` points from `from`, when that is an `https:` URL.
fn redirect_uri(from: &Uri, location: &str) -> Option<Uri> {
    let Ok(base) = url::Url::parse(&from.to_string()) else {
        return None;
    };
    let Ok(target) = base.join(location.trim()) else {
        return None;
    };
    if target.scheme() != "https" {
        return None;
    }
    // A target that is not a URI is no redirect to follow: the 3xx itself is the answer.
    Uri::try_from(target.as_str()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers from a table of URL to `(status, Location)`; any other URL is a 200. Records what
    /// it was asked.
    #[derive(Default)]
    struct Table {
        answers: HashMap<&'static str, (u16, &'static str)>,
        asked: Mutex<Vec<String>>,
        authorized: Mutex<Vec<bool>>,
    }

    impl Http for Table {
        async fn send(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, HttpError> {
            let url = request.uri().to_string();
            self.asked.lock().unwrap().push(url.clone());
            let authorized = request.headers().contains_key(header::AUTHORIZATION);
            self.authorized.lock().unwrap().push(authorized);
            let (status, location) = self.answers.get(url.as_str()).copied().unwrap_or((200, ""));
            let mut answer = Response::builder().status(status);
            if !location.is_empty() {
                answer = answer.header("Location", location);
            }
            Ok(answer.body(Vec::new()).unwrap())
        }
    }

    fn get(url: &str) -> Request<Vec<u8>> {
        Request::builder().uri(url).body(Vec::new()).unwrap()
    }

    fn table(rows: &[(&'static str, u16, &'static str)]) -> Table {
        Table {
            answers: rows.iter().map(|(u, s, l)| (*u, (*s, *l))).collect(),
            ..Table::default()
        }
    }

    #[tokio::test]
    async fn a_redirect_to_a_server_on_the_persons_own_network_is_followed() {
        let http = table(&[
            ("https://example.test/a", 302, "https://192.168.1.5:8443/b"),
            ("https://192.168.1.5:8443/b", 301, "/c"),
        ]);
        let answer = follow(&http, get("https://example.test/a")).await.unwrap();
        assert_eq!(answer.status().as_u16(), 200);
        assert_eq!(
            *http.asked.lock().unwrap(),
            [
                "https://example.test/a",
                "https://192.168.1.5:8443/b",
                "https://192.168.1.5:8443/c"
            ]
        );
    }

    #[tokio::test]
    async fn a_redirect_to_plain_http_is_the_answer_and_is_not_followed() {
        let http = table(&[("https://example.test/a", 302, "http://example.test/x")]);
        let answer = follow(&http, get("https://example.test/a")).await.unwrap();
        assert_eq!(answer.status().as_u16(), 302);
        assert_eq!(
            http.asked.lock().unwrap().len(),
            1,
            "nothing else was asked"
        );
    }

    #[tokio::test]
    async fn a_fourth_hop_is_refused() {
        let http = table(&[
            ("https://example.test/0", 302, "/1"),
            ("https://example.test/1", 302, "/2"),
            ("https://example.test/2", 302, "/3"),
            ("https://example.test/3", 302, "/4"),
        ]);
        let answer = follow(&http, get("https://example.test/0")).await.unwrap();
        assert_eq!(
            answer.status().as_u16(),
            302,
            "the redirect past the limit is the answer"
        );
        assert_eq!(
            http.asked.lock().unwrap().len(),
            4,
            "the first request and three hops"
        );
    }

    #[tokio::test]
    async fn credentials_go_to_the_same_origin_only() {
        let http = table(&[
            ("https://example.test/a", 302, "/b"),
            (
                "https://example.test/b",
                302,
                "https://other.example.test/c",
            ),
            (
                "https://other.example.test/c",
                302,
                "https://example.test/d",
            ),
        ]);
        let mut request = get("https://example.test/a");
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, "Basic eDp5".parse().unwrap());
        follow(&http, request).await.unwrap();
        assert_eq!(
            *http.authorized.lock().unwrap(),
            [true, true, false, false],
            "dropped at the first other origin, and not put back on the way home"
        );
    }

    #[tokio::test]
    async fn a_redirect_with_no_location_is_the_answer() {
        let http = table(&[("https://example.test/a", 302, "")]);
        let answer = follow(&http, get("https://example.test/a")).await.unwrap();
        assert_eq!(answer.status().as_u16(), 302);
    }
}
