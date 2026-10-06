//! Brand logos (BIMI): a domain's logo, looked up in DNS, fetched over HTTPS, checked against its
//! Verified Mark Certificate, drawn to a PNG and cached.
//!
//! What the records and the certificate mean is `mail_mime::bimi`'s; this asks and fetches, in
//! an order that stops at the first answer that rules a logo out:
//!
//! 1. the assertion record, TXT at `default._bimi.<domain>`, else at the organisational
//!    domain's (the draft's fallback);
//! 2. the DMARC policy, `_dmarc.<domain>` and the organisational domain's: at enforcement, or no
//!    logo — without it, anybody can send as the domain and borrow its logo;
//! 3. no `a=`, no logo, and the SVG is never fetched: a self-asserted logo is a picture anyone
//!    with a domain can publish;
//! 4. the certificate (`a=`), which must vouch for the logo against the trust anchors given;
//! 5. the SVG (`l=`), which must be the certificate's own.
//!
//! Whether a message may ask at all — the user's setting, and DMARC passing for its From
//! domain in the receiving server's believed `Authentication-Results` — is the caller's, which
//! alone holds the message. HTTPS only, redirects included, every fetch capped in size and time.
//! The SVG is never shown as a document: [`draw`] turns it into pixels with nothing external
//! loaded.

mod cache;
mod draw;

pub use cache::{Cached, cached, remember};
pub use draw::{SIZE, draw};

use crate::lookup::{Miss, SystemDns};
use chrono::{DateTime, Utc};
use mail_mime::bimi::{self, BimiRecord, MarkProblem, vmc};
use mail_mime::smime::Cert;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

/// How long one DNS query or HTTP request may take.
pub const PER_REQUEST: Duration = Duration::from_secs(5);

/// How long a whole lookup may take.
pub const TOTAL: Duration = Duration::from_secs(20);

/// The largest evidence document read. A mark certificate with its chain and a 32 KiB logo is
/// well under this.
const MAX_EVIDENCE: usize = 128 * 1024;

/// The most redirects followed.
const REDIRECTS: usize = 3;

/// The TXT answers a BIMI lookup needs. A trait so a test answers from a table, and counts.
pub trait Txt {
    /// The TXT records at `name`, each one's strings joined.
    fn txt(&self, name: &str) -> impl Future<Output = Result<Vec<String>, Miss>> + Send;
}

impl Txt for SystemDns {
    async fn txt(&self, name: &str) -> Result<Vec<String>, Miss> {
        self.txt_records(name).await
    }
}

/// A client for BIMI fetches: HTTPS only, [`PER_REQUEST`], redirects only to `https:`.
///
/// A builder so a test can trust its own certificate and point names at a local listener.
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

/// What a lookup is done with.
#[derive(Debug)]
pub struct Lookup<'a, D> {
    pub dns: &'a D,
    pub http: &'a reqwest::Client,
    /// The mark verifying authorities' roots a certificate must chain to.
    pub anchors: &'a [Cert],
    pub now: DateTime<Utc>,
}

/// Why a domain shows no logo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoLogo {
    /// Not a domain name this looks up.
    NotADomain,
    /// No BIMI record, at the domain or its organisational domain.
    NoRecord,
    /// A record that cannot be used.
    BadRecord(bimi::RecordError),
    /// The record declines to name a logo.
    Declined,
    /// The DMARC policy is not at enforcement.
    NotEnforced,
    /// No evidence document: a self-asserted logo, never shown.
    NoEvidence,
    /// The certificate does not vouch for a logo.
    Evidence(MarkProblem),
    /// The SVG at `l=` is not the one the certificate carries.
    NotTheCertified,
    /// The SVG is not one this draws: not SVG Tiny PS, or something in it would load.
    Refused(String),
    /// Something did not answer (DNS, a connection, a timeout, an HTTP error). Not remembered:
    /// asked again next time.
    Unreachable(String),
}

impl NoLogo {
    /// Whether the answer is the domain's, and so worth remembering; a failure to ask is not.
    pub fn settled(&self) -> bool {
        !matches!(self, NoLogo::Unreachable(_))
    }
}

/// `domain`'s logo as a PNG of [`SIZE`] pixels: from the cache under `cache_dir` while fresh,
/// else looked up, checked, drawn and remembered. `None` when it has none; a domain found to
/// have none is remembered too, for [`cache::NONE_FOR`].
pub async fn logo<D: Txt + Sync>(
    lookup: &Lookup<'_, D>,
    cache_dir: &Path,
    domain: &str,
) -> Option<Vec<u8>> {
    let dir = cache_dir.to_owned();
    let asked = domain.to_owned();
    let now = lookup.now;
    let had = tokio::task::spawn_blocking(move || cached(&dir, &asked, now))
        .await
        .ok()?;
    match had {
        Cached::Logo(png) => return Some(png),
        Cached::None => return None,
        Cached::Unknown => {}
    }
    let answer = match tokio::time::timeout(TOTAL, png_for(lookup, domain)).await {
        Ok(answer) => answer,
        Err(_) => Err(NoLogo::Unreachable("the lookup took too long".to_owned())),
    };
    if answer.as_ref().is_err_and(|why| !why.settled()) {
        return None;
    }
    let dir = cache_dir.to_owned();
    let domain = domain.to_owned();
    let kept = answer.clone().ok();
    let _ =
        tokio::task::spawn_blocking(move || remember(&dir, &domain, kept.as_deref(), now)).await;
    answer.ok()
}

/// [`find`], then [`draw`] off the async threads.
async fn png_for<D: Txt + Sync>(lookup: &Lookup<'_, D>, domain: &str) -> Result<Vec<u8>, NoLogo> {
    let svg = find(lookup, domain).await?;
    tokio::task::spawn_blocking(move || draw(&svg))
        .await
        .map_err(|e| NoLogo::Refused(e.to_string()))?
}

/// `domain`'s certified SVG, by the steps in this module's comment. No cache.
pub async fn find<D: Txt + Sync>(lookup: &Lookup<'_, D>, domain: &str) -> Result<Vec<u8>, NoLogo> {
    let domain = normalise(domain).ok_or(NoLogo::NotADomain)?;
    let org = psl::domain_str(&domain).unwrap_or(&domain).to_owned();
    let same = org == domain;

    let record = match record_at(lookup.dns, &domain).await? {
        Some(record) => record,
        None if !same => record_at(lookup.dns, &org).await?.ok_or(NoLogo::NoRecord)?,
        None => return Err(NoLogo::NoRecord),
    };

    let author = dmarc_at(lookup.dns, &domain).await?;
    let organisational = if same {
        author.clone()
    } else {
        dmarc_at(lookup.dns, &org).await?
    };
    if !bimi::enforced(author.as_ref(), organisational.as_ref(), same) {
        return Err(NoLogo::NotEnforced);
    }

    let Some(evidence) = record.evidence else {
        return Err(NoLogo::NoEvidence);
    };
    let Some(location) = record.location else {
        return Err(NoLogo::Declined);
    };
    let pem = fetch(lookup.http, &evidence, MAX_EVIDENCE).await?;
    let certified = match bimi::verified_logo(&pem, &domain, lookup.anchors, lookup.now) {
        Err(MarkProblem::NotForDomain) if !same => {
            bimi::verified_logo(&pem, &org, lookup.anchors, lookup.now)
        }
        other => other,
    }
    .map_err(NoLogo::Evidence)?;
    let fetched = fetch(lookup.http, &location, vmc::MAX_SVG).await?;
    if !bimi::same_logo(&fetched, &certified) {
        return Err(NoLogo::NotTheCertified);
    }
    Ok(certified)
}

/// The BIMI record at `domain`'s default selector, if it has one.
async fn record_at<D: Txt>(dns: &D, domain: &str) -> Result<Option<BimiRecord>, NoLogo> {
    let txts = answered(dns.txt(&format!("default._bimi.{domain}")).await)?;
    bimi::pick_record(&txts)
        .transpose()
        .map_err(NoLogo::BadRecord)
}

/// The DMARC record at `domain`, if it has one.
async fn dmarc_at<D: Txt>(dns: &D, domain: &str) -> Result<Option<bimi::DmarcRecord>, NoLogo> {
    let txts = answered(dns.txt(&format!("_dmarc.{domain}")).await)?;
    Ok(bimi::pick_dmarc(&txts))
}

/// No records is an answer; no answer is not.
fn answered(result: Result<Vec<String>, Miss>) -> Result<Vec<String>, NoLogo> {
    match result {
        Ok(txts) => Ok(txts),
        Err(Miss::Absent) => Ok(Vec::new()),
        Err(Miss::Unreachable(why)) => Err(NoLogo::Unreachable(why)),
    }
}

/// The body at `url`, at most `max` bytes. Anything but a 2xx is unreachable.
async fn fetch(http: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, NoLogo> {
    let unreachable = |why: String| NoLogo::Unreachable(format!("{url}: {why}"));
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|e| unreachable(e.to_string()))?;
    if !response.status().is_success() {
        return Err(unreachable(response.status().to_string()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > max as u64)
    {
        return Err(NoLogo::Refused(format!("{url}: larger than {max} bytes")));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| unreachable(e.to_string()))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > max {
            return Err(NoLogo::Refused(format!("{url}: larger than {max} bytes")));
        }
    }
    Ok(body)
}

/// `domain` lower case without a trailing dot, when it is a DNS name of at least two labels,
/// each letters, digits and inner hyphens. Anything else — an address literal, a name with a
/// space or a slash — is not looked up, and never becomes a file name in the cache.
pub fn normalise(domain: &str) -> Option<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').collect();
    let label_ok = |label: &&str| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    (domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(label_ok)
        && !labels
            .last()
            .is_some_and(|tld| tld.bytes().all(|b| b.is_ascii_digit())))
    .then_some(domain)
}

#[cfg(test)]
mod tests {
    use super::normalise;

    #[test]
    fn only_a_dns_name_is_looked_up() {
        let cases: &[(&str, Option<&str>)] = &[
            ("Brand.Example", Some("brand.example")),
            ("mail.brand.example.", Some("mail.brand.example")),
            ("xn--bcher-kva.example", Some("xn--bcher-kva.example")),
            ("example", None),
            ("192.0.2.1", None),
            ("[192.0.2.1]", None),
            ("../etc/passwd", None),
            ("brand example.com", None),
            ("-brand.example", None),
            ("brand-.example", None),
            ("bräнд.example", None),
            ("", None),
        ];
        for (domain, want) in cases {
            assert_eq!(normalise(domain).as_deref(), *want, "{domain:?}");
        }
    }
}
