//! The two DNS records a brand logo rests on: the BIMI assertion record and the DMARC policy
//! record, read from their TXT strings, and whether a message may show a logo at all.
//!
//! BIMI's assertion record (draft-brand-indicators-for-message-identification, "BIMI Assertion
//! Record Definition") is a tag list like DKIM's: `v=BIMI1; l=<https URL of an SVG>; a=<https
//! URL of the evidence document>`. `v` must come first and be exactly `BIMI1`; tags this reader
//! does not know are ignored. DMARC's (RFC 7489 §6.3) is the same shape, `v=DMARC1; p=reject;
//! sp=…; pct=…`.

use crate::auth::{AuthResults, Verdict};
pub use crate::error::RecordError;

/// A domain's BIMI assertion record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BimiRecord {
    /// Where the logo is: an `https:` URL. `None` when `l=` is empty or absent.
    pub location: Option<String>,
    /// Where the evidence document (a Verified Mark Certificate) is: an `https:` URL. `None`
    /// when `a=` is empty or absent, which is a self-asserted logo that nobody vouched for.
    pub evidence: Option<String>,
}

/// One TXT string, as a BIMI record.
pub fn parse_record(txt: &str) -> Result<BimiRecord, RecordError> {
    let tags = tag_list(txt).ok_or(RecordError::NotBimi)?;
    match tags.first() {
        Some((name, value)) if name == "v" && value == "BIMI1" => {}
        _ => return Err(RecordError::NotBimi),
    }
    let mut location = None;
    let mut evidence = None;
    for (name, value) in tags.iter().skip(1) {
        let slot = match name.as_str() {
            "l" => &mut location,
            "a" => &mut evidence,
            "v" => return Err(RecordError::Repeated(name.clone())),
            _ => continue,
        };
        if slot.is_some() {
            return Err(RecordError::Repeated(name.clone()));
        }
        *slot = Some(https_or_empty(value)?);
    }
    Ok(BimiRecord {
        location: location.flatten(),
        evidence: evidence.flatten(),
    })
}

/// The one BIMI record among the TXT strings found at a name. `None` when none of them opens
/// with `v=BIMI1`, and when more than one does: the draft has a receiver treat several records
/// as none, since which one the domain meant cannot be known.
pub fn pick_record(txts: &[String]) -> Option<Result<BimiRecord, RecordError>> {
    let mut candidates = txts
        .iter()
        .filter(|txt| !matches!(parse_record(txt), Err(RecordError::NotBimi)));
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(parse_record(first))
}

/// `value` as `Some(url)`, `None` when empty, or an error when it is anything but one `https:`
/// URL. A comma-separated list (an older draft allowed two) is refused rather than guessed at.
fn https_or_empty(value: &str) -> Result<Option<String>, RecordError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let bad = || RecordError::NotHttps(value.to_owned());
    if value.contains(',') || value.chars().any(char::is_whitespace) {
        return Err(bad());
    }
    let url = url::Url::parse(value).map_err(|_| bad())?;
    if url.scheme() != "https" || url.host_str().is_none_or(str::is_empty) {
        return Err(bad());
    }
    Ok(Some(url.to_string()))
}

/// `name=value; name=value`, names lower case and values trimmed. `None` when a part has no
/// `=`, or a name is empty.
fn tag_list(txt: &str) -> Option<Vec<(String, String)>> {
    txt.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=')?;
            let name = name.trim().to_ascii_lowercase();
            (!name.is_empty()).then(|| (name, value.trim().to_owned()))
        })
        .collect()
}

/// What a DMARC record asks receivers to do with mail that fails (RFC 7489 §6.3 `p` and `sp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    None,
    Quarantine,
    Reject,
}

/// A domain's DMARC policy record: the parts BIMI reads.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DmarcRecord {
    /// `p`: the domain's own policy.
    pub policy: Disposition,
    /// `sp`: its subdomains', when it states one.
    pub subdomains: Option<Disposition>,
    /// `pct`: the share of failing mail the policy applies to, 0 to 100; 100 when absent.
    pub percent: u8,
}

/// One TXT string, as a DMARC record. `None` when it is not one, or its `p` is missing or not a
/// word DMARC defines (§6.6.3 would read a missing `p` as `none` in some cases; as far as a logo
/// is concerned that is the same answer).
pub fn parse_dmarc(txt: &str) -> Option<DmarcRecord> {
    let tags = tag_list(txt)?;
    match tags.first() {
        Some((name, value)) if name == "v" && value == "DMARC1" => {}
        _ => return None,
    }
    let find = |tag: &str| {
        tags.iter()
            .find(|(name, _)| name == tag)
            .map(|(_, value)| value.as_str())
    };
    let disposition = |word: &str| match word.to_ascii_lowercase().as_str() {
        "none" => Some(Disposition::None),
        "quarantine" => Some(Disposition::Quarantine),
        "reject" => Some(Disposition::Reject),
        _ => None,
    };
    let policy = disposition(find("p")?)?;
    let subdomains = match find("sp") {
        Some(word) => Some(disposition(word)?),
        None => None,
    };
    let percent = match find("pct") {
        Some(n) => n.parse::<u8>().ok().filter(|n| *n <= 100)?,
        None => 100,
    };
    Some(DmarcRecord {
        policy,
        subdomains,
        percent,
    })
}

/// The DMARC record among the TXT strings at `_dmarc.<domain>`: exactly one (RFC 7489 §6.6.3
/// has a receiver use no policy when there are several).
pub fn pick_dmarc(txts: &[String]) -> Option<DmarcRecord> {
    let mut found = txts.iter().filter_map(|txt| parse_dmarc(txt));
    let one = found.next()?;
    found.next().is_none().then_some(one)
}

/// Whether the DMARC policy that governs the author domain is at enforcement, as BIMI requires
/// before any logo is shown: `quarantine` applied to all mail, or `reject`.
///
/// `author` is the record at `_dmarc.<author domain>`, `organisational` the one at the
/// organisational domain's (the same record when the author domain is the organisational
/// domain). The author's own record governs when there is one; otherwise the organisational
/// domain's subdomain policy, else its policy. The organisational domain's own `p` must not be
/// `none` either, nor its `sp` when the author is a subdomain.
pub fn enforced(
    author: Option<&DmarcRecord>,
    organisational: Option<&DmarcRecord>,
    author_is_organisational: bool,
) -> bool {
    let strict = |disposition: Disposition, percent: u8| match disposition {
        Disposition::Reject => true,
        Disposition::Quarantine => percent == 100,
        Disposition::None => false,
    };
    let governing = match (author, organisational) {
        (Some(own), _) => strict(own.policy, own.percent),
        (None, Some(org)) if !author_is_organisational => {
            strict(org.subdomains.unwrap_or(org.policy), org.percent)
        }
        _ => false,
    };
    let org_ok = match organisational {
        Some(org) => {
            org.policy != Disposition::None
                && (author_is_organisational || org.subdomains != Some(Disposition::None))
        }
        None => author_is_organisational,
    };
    governing && org_ok
}

/// Whether the receiving server's word on a message lets it show the From domain's logo: DMARC
/// passed, and passed for that domain (`header.from`, compared whole). Anything less — DMARC not
/// checked, a pass the field does not tie to a domain, a pass for another domain — is no.
pub fn dmarc_passed_for(results: &AuthResults, from_domain: &str) -> bool {
    let from_domain = from_domain.trim_end_matches('.');
    results.dmarc.as_ref().is_some_and(|check| {
        check.verdict == Verdict::Pass
            && check
                .domain
                .as_deref()
                .is_some_and(|d| d.trim_end_matches('.').eq_ignore_ascii_case(from_domain))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Check;

    fn record(l: Option<&str>, a: Option<&str>) -> Result<BimiRecord, RecordError> {
        Ok(BimiRecord {
            location: l.map(str::to_owned),
            evidence: a.map(str::to_owned),
        })
    }

    #[test]
    fn a_bimi_record_is_read_by_the_drafts_grammar() {
        let svg = "https://brand.example/logo.svg";
        let pem = "https://brand.example/vmc.pem";
        let cases: &[(&str, &str, Result<BimiRecord, RecordError>)] = &[
            (
                "both",
                "v=BIMI1; l=https://brand.example/logo.svg; a=https://brand.example/vmc.pem",
                record(Some(svg), Some(pem)),
            ),
            (
                "no evidence",
                "v=BIMI1; l=https://brand.example/logo.svg",
                record(Some(svg), None),
            ),
            (
                "empty evidence",
                "v=BIMI1; l=https://brand.example/logo.svg; a=;",
                record(Some(svg), None),
            ),
            ("declined", "v=BIMI1; l=; a=;", record(None, None)),
            (
                "spaces and an unknown tag",
                "  v = BIMI1 ;  x=whatever;l = https://brand.example/logo.svg ; ",
                record(Some(svg), None),
            ),
            (
                "tag names are any case",
                "v=BIMI1; L=https://brand.example/logo.svg; A=https://brand.example/vmc.pem",
                record(Some(svg), Some(pem)),
            ),
            (
                "not first",
                "l=https://brand.example/logo.svg; v=BIMI1",
                Err(RecordError::NotBimi),
            ),
            (
                "another version",
                "v=BIMI2; l=https://brand.example/logo.svg",
                Err(RecordError::NotBimi),
            ),
            (
                "lower-case version",
                "v=bimi1; l=https://brand.example/logo.svg",
                Err(RecordError::NotBimi),
            ),
            (
                "spf",
                "v=spf1 include:_spf.example -all",
                Err(RecordError::NotBimi),
            ),
            (
                "plain http",
                "v=BIMI1; l=http://brand.example/logo.svg",
                Err(RecordError::NotHttps(
                    "http://brand.example/logo.svg".to_owned(),
                )),
            ),
            (
                "two locations",
                "v=BIMI1; l=https://a.example/1.svg,https://b.example/2.svg",
                Err(RecordError::NotHttps(
                    "https://a.example/1.svg,https://b.example/2.svg".to_owned(),
                )),
            ),
            (
                "a data url",
                "v=BIMI1; l=data:image/svg+xml,<svg/>",
                Err(RecordError::NotHttps(
                    "data:image/svg+xml,<svg/>".to_owned(),
                )),
            ),
            (
                "repeated",
                "v=BIMI1; l=https://brand.example/logo.svg; l=https://brand.example/logo.svg",
                Err(RecordError::Repeated("l".to_owned())),
            ),
            ("no equals", "v=BIMI1; garbage", Err(RecordError::NotBimi)),
        ];
        for (name, txt, want) in cases {
            assert_eq!(&parse_record(txt), want, "case: {name}");
        }
    }

    #[test]
    fn several_records_are_none() {
        let one = "v=BIMI1; l=https://brand.example/logo.svg".to_owned();
        let spf = "v=spf1 -all".to_owned();
        assert_eq!(
            pick_record(&[spf.clone(), one.clone()]),
            Some(record(Some("https://brand.example/logo.svg"), None))
        );
        assert_eq!(pick_record(&[one.clone(), one]), None);
        assert_eq!(pick_record(&[spf]), None);
        assert_eq!(pick_record(&[]), None);
    }

    fn dmarc(p: Disposition, sp: Option<Disposition>, pct: u8) -> DmarcRecord {
        DmarcRecord {
            policy: p,
            subdomains: sp,
            percent: pct,
        }
    }

    #[test]
    fn a_dmarc_record_is_read_for_its_policy() {
        use Disposition as D;
        let cases: &[(&str, Option<DmarcRecord>)] = &[
            ("v=DMARC1; p=reject", Some(dmarc(D::Reject, None, 100))),
            (
                "v=DMARC1; p=quarantine; sp=none; pct=50; rua=mailto:d@example.com",
                Some(dmarc(D::Quarantine, Some(D::None), 50)),
            ),
            ("v=DMARC1;p=NONE", Some(dmarc(D::None, None, 100))),
            ("v=DMARC1; rua=mailto:d@example.com", None),
            ("v=DMARC1; p=bounce", None),
            ("v=DMARC1; p=reject; pct=101", None),
            ("v=DMARC1; p=reject; sp=maybe", None),
            ("p=reject; v=DMARC1", None),
            ("v=spf1 -all", None),
        ];
        for (txt, want) in cases {
            assert_eq!(&parse_dmarc(txt), want, "{txt}");
        }
        let two = [
            "v=DMARC1; p=reject".to_owned(),
            "v=DMARC1; p=none".to_owned(),
        ];
        assert_eq!(pick_dmarc(&two), None, "two records are none");
    }

    #[test]
    fn only_a_policy_at_enforcement_allows_a_logo() {
        use Disposition as D;
        let reject = dmarc(D::Reject, None, 100);
        let quarantine = dmarc(D::Quarantine, None, 100);
        let partial = dmarc(D::Quarantine, None, 50);
        let none = dmarc(D::None, None, 100);
        let loose_subs = dmarc(D::Reject, Some(D::None), 100);
        let strict_subs = dmarc(D::None, Some(D::Reject), 100);
        // (case, the author's record, the organisational domain's, the same domain, allowed)
        type Case<'a> = (
            &'a str,
            Option<&'a DmarcRecord>,
            Option<&'a DmarcRecord>,
            bool,
            bool,
        );
        let cases: &[Case] = &[
            ("org reject", Some(&reject), Some(&reject), true, true),
            (
                "org quarantine",
                Some(&quarantine),
                Some(&quarantine),
                true,
                true,
            ),
            (
                "org quarantine at half",
                Some(&partial),
                Some(&partial),
                true,
                false,
            ),
            ("org none", Some(&none), Some(&none), true, false),
            ("no record", None, None, true, false),
            ("sub under org reject", None, Some(&reject), false, true),
            (
                "sub under org sp=none",
                None,
                Some(&loose_subs),
                false,
                false,
            ),
            (
                "sub under org p=none sp=reject",
                None,
                Some(&strict_subs),
                false,
                false,
            ),
            (
                "sub with its own reject",
                Some(&reject),
                Some(&reject),
                false,
                true,
            ),
            (
                "sub with its own none",
                Some(&none),
                Some(&reject),
                false,
                false,
            ),
            ("sub with no org record", Some(&reject), None, false, false),
        ];
        for (name, author, org, same, want) in cases {
            assert_eq!(enforced(*author, *org, *same), *want, "case: {name}");
        }
    }

    #[test]
    fn only_a_dmarc_pass_for_the_from_domain_counts() {
        let results = |verdict, domain: Option<&str>| AuthResults {
            authserv_id: Some("mx.provider.example".to_owned()),
            dmarc: Some(Check {
                verdict,
                domain: domain.map(str::to_owned),
            }),
            ..AuthResults::default()
        };
        let cases = [
            ("pass", results(Verdict::Pass, Some("brand.example")), true),
            (
                "pass, any case",
                results(Verdict::Pass, Some("Brand.Example")),
                true,
            ),
            ("fail", results(Verdict::Fail, Some("brand.example")), false),
            ("pass for no domain", results(Verdict::Pass, None), false),
            (
                "pass for another",
                results(Verdict::Pass, Some("evil-brand.example")),
                false,
            ),
            (
                "pass for a parent",
                results(Verdict::Pass, Some("example")),
                false,
            ),
        ];
        for (name, results, want) in cases {
            assert_eq!(
                dmarc_passed_for(&results, "brand.example"),
                want,
                "case: {name}"
            );
        }
        let unchecked = AuthResults::default();
        assert!(!dmarc_passed_for(&unchecked, "brand.example"));
    }
}
