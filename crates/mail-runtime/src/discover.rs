//! Account discovery's network half: fetching configuration documents and asking DNS.
//!
//! What an answer means is `mail_proto::discover`'s; this only fetches, in order, and stops at
//! the first source whose answer is usable:
//!
//! 1. `https://autoconfig.<domain>/mail/config-v1.1.xml?emailaddress=<address>`, then
//!    `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml` — the domain's own, and the
//!    only request that carries the address, to the domain that owns it;
//! 2. the ISPDB's document for the domain (the domain only, never the address);
//! 3. SRV records `_imaps._tcp`, `_pop3s._tcp` and `_submissions._tcp`;
//! 4. the MX record, which either names a provider the preset table knows or leads to the
//!    ISPDB document for its host's domain.
//!
//! HTTPS only, with certificate verification (reqwest's default, which nothing here turns off),
//! and no plain-HTTP fallback: a configuration document fetched in the clear could name any
//! server at all. Redirects are followed only to `https:`. Each request has [`PER_REQUEST`] and
//! the whole search [`TOTAL`], so an unreachable network is reported in seconds, not minutes.
//!
//! Nothing is sent to a discovered server here. The [`Found`] goes back to the caller, which
//! shows it and asks before anything is configured.

use mail_proto::discover::{
    self, Found, MxLead, MxRecord, Source, SrvRecord, Unusable, autoconfig,
};
use std::fmt;
use std::future::Future;
use std::time::Duration;

/// Where the ISPDB is served.
pub const ISPDB: &str = "https://autoconfig.thunderbird.net/v1.1/";

/// How long one HTTP request or DNS query may take.
pub const PER_REQUEST: Duration = Duration::from_secs(4);

/// How long the whole search may take.
pub const TOTAL: Duration = Duration::from_secs(25);

/// The most a configuration document may weigh. Real ones are a few kilobytes.
const MAX_DOCUMENT: usize = 256 * 1024;

/// The most redirects to follow.
const REDIRECTS: usize = 3;

/// The DNS answers discovery needs. A trait because tests answer from a table: no test may ask a
/// real resolver, and the system one is the only other implementation.
pub trait Dns {
    fn srv(&self, name: &str) -> impl Future<Output = Result<Vec<SrvRecord>, Miss>> + Send;
    fn mx(&self, name: &str) -> impl Future<Output = Result<Vec<MxRecord>, Miss>> + Send;
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
            .map_err(|e| crate::RuntimeError::Connect(format!("no DNS configuration: {e}")))?;
        builder.options_mut().timeout = PER_REQUEST;
        builder.options_mut().attempts = 1;
        builder
            .build()
            .map(SystemDns)
            .map_err(|e| crate::RuntimeError::Connect(format!("cannot start a resolver: {e}")))
    }
}

impl Dns for SystemDns {
    async fn srv(&self, name: &str) -> Result<Vec<SrvRecord>, Miss> {
        use hickory_resolver::proto::rr::RData;
        let lookup = self.0.srv_lookup(name).await.map_err(dns_miss)?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::SRV(srv) => Some(SrvRecord {
                    priority: srv.priority,
                    weight: srv.weight,
                    port: srv.port,
                    target: srv.target.to_utf8(),
                }),
                _ => None,
            })
            .collect())
    }

    async fn mx(&self, name: &str) -> Result<Vec<MxRecord>, Miss> {
        use hickory_resolver::proto::rr::RData;
        let lookup = self.0.mx_lookup(name).await.map_err(dns_miss)?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::MX(mx) => Some(MxRecord {
                    preference: mx.preference,
                    exchange: mx.exchange.to_utf8(),
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

/// Why one step found nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// The source answered: there is nothing there (404, no records).
    Absent,
    /// No answer: DNS, the connection or TLS failed, or it timed out.
    Unreachable(String),
    /// An answer that is not a configuration: another status, or a document that did not parse.
    Malformed(String),
    /// A configuration that cannot be used, and why.
    Unusable(Unusable),
}

/// One step of the search and what came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tried {
    /// The URL fetched or the DNS name asked about.
    pub what: String,
    pub miss: Miss,
}

/// The search found nothing usable. Every step is listed, so the user sees what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound {
    pub tried: Vec<Tried>,
    /// The search ran out of [`TOTAL`] before every step was tried.
    pub timed_out: bool,
}

impl NotFound {
    /// Nothing answered at all: most likely this computer is offline.
    pub fn offline(&self) -> bool {
        !self.tried.is_empty()
            && self
                .tried
                .iter()
                .all(|t| matches!(t.miss, Miss::Unreachable(_)))
    }

    /// The unusable configurations that were found, which say more than "nothing found".
    pub fn unusable(&self) -> impl Iterator<Item = &Unusable> {
        self.tried.iter().filter_map(|t| match &t.miss {
            Miss::Unusable(why) => Some(why),
            _ => None,
        })
    }
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.offline() {
            f.write_str("could not reach anything to ask; is this computer offline?")?;
        } else {
            f.write_str("no usable configuration was found")?;
        }
        if self.timed_out {
            write!(f, " (gave up after {} seconds)", TOTAL.as_secs())?;
        }
        for tried in &self.tried {
            let why = match &tried.miss {
                Miss::Absent => "nothing there".to_owned(),
                Miss::Unreachable(why) => format!("no answer: {why}"),
                Miss::Malformed(why) => why.clone(),
                Miss::Unusable(why) => why.to_string(),
            };
            write!(f, "\n  {}: {why}", tried.what)?;
        }
        Ok(())
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

/// Where the sources are. Only the ISPDB's address is configurable, and only so a test can serve
/// it; the domain's own URLs are fixed by the domain.
#[derive(Debug, Clone)]
pub struct Sources {
    /// Ends in `/`; the domain is appended.
    pub ispdb: String,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            ispdb: ISPDB.to_owned(),
        }
    }
}

/// Search every source in order for `address`, stopping at the first usable answer.
pub async fn discover(
    address: &str,
    http: &reqwest::Client,
    dns: &impl Dns,
    sources: &Sources,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Found, NotFound> {
    let mut tried = Vec::new();
    match tokio::time::timeout(TOTAL, search(address, http, dns, sources, now, &mut tried)).await {
        Ok(Some(found)) => Ok(found),
        Ok(None) => Err(NotFound {
            tried,
            timed_out: false,
        }),
        Err(_) => Err(NotFound {
            tried,
            timed_out: true,
        }),
    }
}

async fn search(
    address: &str,
    http: &reqwest::Client,
    dns: &impl Dns,
    sources: &Sources,
    now: chrono::DateTime<chrono::Utc>,
    tried: &mut Vec<Tried>,
) -> Option<Found> {
    let Some(domain) = discover::domain_of(address) else {
        tried.push(Tried {
            what: address.to_owned(),
            miss: Miss::Malformed("not an email address".to_owned()),
        });
        return None;
    };

    // 1. The domain's own document.
    let own = [
        with_query(
            &format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
            address,
        ),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    for url in own {
        if let Some(preset) = document(http, &url, address, now, tried).await {
            return Some(Found {
                source: Source::DomainAutoconfig,
                preset,
            });
        }
    }

    // 2. The ISPDB.
    let ispdb = format!("{}{domain}", sources.ispdb);
    if let Some(preset) = document(http, &ispdb, address, now, tried).await {
        return Some(Found {
            source: Source::Ispdb,
            preset,
        });
    }

    // 3. SRV. Asked together: they are independent, and a slow resolver should cost one
    // timeout here rather than three.
    let names = [
        format!("_imaps._tcp.{domain}"),
        format!("_pop3s._tcp.{domain}"),
        format!("_submissions._tcp.{domain}"),
    ];
    let (imaps, pop3s, submissions) =
        tokio::join!(dns.srv(&names[0]), dns.srv(&names[1]), dns.srv(&names[2]));
    let mut records = |name: &str, answer: Result<Vec<SrvRecord>, Miss>| match answer {
        Ok(records) => records,
        Err(miss) => {
            tried.push(Tried {
                what: format!("SRV {name}"),
                miss,
            });
            Vec::new()
        }
    };
    let imaps = records(&names[0], imaps);
    let pop3s = records(&names[1], pop3s);
    let submissions = records(&names[2], submissions);
    if !(imaps.is_empty() && pop3s.is_empty() && submissions.is_empty()) {
        match discover::from_srv(address, &imaps, &pop3s, &submissions, now) {
            Ok(preset) => {
                return Some(Found {
                    source: Source::Srv,
                    preset,
                });
            }
            Err(why) => tried.push(Tried {
                what: format!("SRV records for {domain}"),
                miss: Miss::Unusable(why),
            }),
        }
    }

    // 4. MX.
    let mx = match dns.mx(&domain).await {
        Ok(mx) => mx,
        Err(miss) => {
            tried.push(Tried {
                what: format!("MX {domain}"),
                miss,
            });
            return None;
        }
    };
    let registered = |host: &str| psl::domain_str(host).map(str::to_ascii_lowercase);
    match discover::from_mx(address, &mx, registered, now) {
        Ok(MxLead::Known(found)) => Some(*found),
        Ok(MxLead::AskIspdb { domain, source }) => {
            let url = format!("{}{domain}", sources.ispdb);
            let preset = document(http, &url, address, now, tried).await?;
            Some(Found { source, preset })
        }
        Ok(MxLead::Nothing) => {
            tried.push(Tried {
                what: format!("MX {domain}"),
                miss: Miss::Absent,
            });
            None
        }
        Err(why) => {
            tried.push(Tried {
                what: format!("MX {domain}"),
                miss: Miss::Unusable(why),
            });
            None
        }
    }
}

/// `url?emailaddress=<address>`, encoded.
fn with_query(url: &str, address: &str) -> String {
    match url::Url::parse_with_params(url, [("emailaddress", address)]) {
        Ok(url) => url.to_string(),
        Err(_) => url.to_owned(),
    }
}

/// Fetch one document and choose from it, recording why not when it cannot be used.
async fn document(
    http: &reqwest::Client,
    url: &str,
    address: &str,
    now: chrono::DateTime<chrono::Utc>,
    tried: &mut Vec<Tried>,
) -> Option<mail_domain::presets::Preset> {
    let miss = match fetch(http, url).await {
        Ok(body) => match autoconfig::parse(&body) {
            Ok(config) => match discover::from_autoconfig(&config, address, now) {
                Ok(preset) => return Some(preset),
                Err(why) => Miss::Unusable(why),
            },
            Err(why) => Miss::Malformed(why.to_string()),
        },
        Err(miss) => miss,
    };
    tried.push(Tried {
        what: url.to_owned(),
        miss,
    });
    None
}

/// GET a document, at most [`MAX_DOCUMENT`] of it.
async fn fetch(http: &reqwest::Client, url: &str) -> Result<String, Miss> {
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|e| Miss::Unreachable(error_chain(&e)))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
        return Err(Miss::Absent);
    }
    if !status.is_success() {
        return Err(Miss::Malformed(format!("answered {}", status.as_u16())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Miss::Unreachable(error_chain(&e)))?
    {
        if body.len() + chunk.len() > MAX_DOCUMENT {
            return Err(Miss::Malformed(format!(
                "the document is larger than {} KiB",
                MAX_DOCUMENT / 1024
            )));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| Miss::Malformed("the document is not UTF-8".to_owned()))
}

/// reqwest's error and its causes, which is where "certificate not valid for name" lives.
fn error_chain(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
