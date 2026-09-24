//! What both stores agree on about OpenPGP keys: the order a recipient's keys are offered in.
//!
//! One function both implementations call, rather than an `ORDER BY` beside a `sort_by`, so the
//! parity test compares storage and not two spellings of a preference.

use mail_domain::{KeyTrust, PgpKey};
use std::cmp::Reverse;

/// Best first: a key the user verified, then by how it arrived ([`mail_domain::KeySource::rank`]),
/// then the most recently seen, then by fingerprint so the order is total.
pub(crate) fn preferred(mut keys: Vec<PgpKey>) -> Vec<PgpKey> {
    keys.sort_by_key(|key| {
        (
            Reverse(key.trust == KeyTrust::Verified),
            Reverse(key.source.rank()),
            Reverse(key.last_seen),
            key.fingerprint,
        )
    });
    keys
}
