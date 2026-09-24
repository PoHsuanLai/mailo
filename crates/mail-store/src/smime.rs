//! What both stores agree on about S/MIME certificates: the order an address's certificates are
//! offered in. One function both implementations call, for the reason `pgp.rs` gives.

use mail_domain::{KeyTrust, SmimeCert};
use std::cmp::Reverse;

/// Best first: one the user trusts, then by how it arrived ([`mail_domain::CertSource::rank`]),
/// then the one valid longest, then the most recently seen, then by fingerprint so the order is
/// total.
pub(crate) fn preferred(mut certs: Vec<SmimeCert>) -> Vec<SmimeCert> {
    certs.sort_by_key(|cert| {
        (
            Reverse(cert.trust == KeyTrust::Verified),
            Reverse(cert.source.rank()),
            Reverse(cert.not_after),
            Reverse(cert.last_seen),
            cert.fingerprint,
        )
    });
    certs
}
