//! Whether a signer's certificate is believed: a chain to a trust anchor, dates, purpose, name.
//!
//! RFC 5280 §6's path validation, cut to what mail needs and what can be known offline: each
//! link's signature and dates, each issuer a CA allowed to issue, the end a trust anchor. No
//! revocation — nothing here fetches a CRL or asks an OCSP responder, so a revoked certificate
//! whose chain is otherwise sound is believed. No name or policy constraints.

use super::cert::Cert;
use super::sign::issued;
use chrono::{DateTime, Utc};
use mail_domain::CertProblem;

/// The longest chain followed. Real ones are three or four long.
const MAX_CHAIN: usize = 8;

/// What is wrong with `signer` as the certificate behind a signature checked for `at`, from the
/// address `from`: empty when nothing is.
///
/// Issuers are looked for among `anchors` first, then in `pool` — the certificates the message
/// carried and those the client holds. A certificate that is itself an anchor needs no issuer.
pub(crate) fn problems(
    signer: &Cert,
    pool: &[Cert],
    anchors: &[Cert],
    at: DateTime<Utc>,
    from: Option<&str>,
) -> Vec<CertProblem> {
    let mut out = Vec::new();
    let mut note = |problem: CertProblem| {
        if !out.contains(&problem) {
            out.push(problem);
        }
    };
    if let Some(problem) = dated(signer, at) {
        note(problem);
    }
    if !signer.may_sign_mail() {
        note(CertProblem::NotForEmail);
    }
    match from {
        Some(from) if signer.is_for(from) => {}
        Some(from) => note(CertProblem::NotFrom {
            from: from.trim().to_owned(),
        }),
        None => note(CertProblem::NotFrom {
            from: String::new(),
        }),
    }
    match chain(signer, pool, anchors) {
        Some(issuers) => {
            for issuer in issuers {
                if let Some(problem) = dated(issuer, at) {
                    note(problem);
                }
            }
        }
        None => note(CertProblem::Untrusted),
    }
    out
}

/// The certificate's dates against `at`.
fn dated(cert: &Cert, at: DateTime<Utc>) -> Option<CertProblem> {
    if at > cert.not_after() {
        Some(CertProblem::Expired {
            not_after: cert.not_after(),
        })
    } else if at < cert.not_before() {
        Some(CertProblem::NotYetValid {
            not_before: cert.not_before(),
        })
    } else {
        None
    }
}

/// The intermediate issuers from `leaf` up to a trust anchor, the anchor itself not included
/// (an anchor is trusted as given, dates and all, as RFC 5280 §6.1.1 treats one). `None` when no
/// chain reaches an anchor.
fn chain<'a>(leaf: &'a Cert, pool: &'a [Cert], anchors: &'a [Cert]) -> Option<Vec<&'a Cert>> {
    let is_anchor = |cert: &Cert| anchors.iter().any(|a| a.der() == cert.der());
    if is_anchor(leaf) {
        return Some(Vec::new());
    }
    let mut path: Vec<&Cert> = Vec::new();
    let mut current = leaf;
    for _ in 0..MAX_CHAIN {
        let issuer_name = &current.inner.tbs_certificate.issuer;
        let named = |candidate: &&Cert| candidate.inner.tbs_certificate.subject == *issuer_name;
        if anchors
            .iter()
            .filter(named)
            .any(|anchor| anchor.may_issue_as_anchor() && issued(current, anchor))
        {
            return Some(path);
        }
        let next = pool.iter().filter(named).find(|candidate| {
            candidate.der() != current.der()
                && !path.iter().any(|p| p.der() == candidate.der())
                && candidate.may_issue()
                && issued(current, candidate)
        })?;
        path.push(next);
        current = next;
    }
    None
}
