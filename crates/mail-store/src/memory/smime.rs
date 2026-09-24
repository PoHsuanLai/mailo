//! S/MIME certificates in the in-memory store. Kept in step with `sqlite/smime.rs`, which the
//! parity tests in `tests/smime_certs.rs` hold it to.

use super::Inner;
use crate::StoreError;
use mail_domain::{CertFingerprint, KeyTrust, SmimeCert};

impl Inner {
    pub(super) fn smime_certs_for(&self, address: &str) -> Vec<SmimeCert> {
        let address = address.trim().to_ascii_lowercase();
        // `json_each(emails) WHERE value = ?`: an exact match on the stored, lower-cased text.
        crate::smime::preferred(
            self.smime_certs
                .values()
                .filter(|c| c.emails.contains(&address))
                .cloned()
                .collect(),
        )
    }

    pub(super) fn put_smime_cert(&mut self, cert: SmimeCert) -> SmimeCert {
        let merged = match self.smime_certs.remove(&cert.fingerprint) {
            Some(held) => held.merged(cert),
            None => cert,
        };
        self.smime_certs.insert(merged.fingerprint, merged.clone());
        merged
    }

    pub(super) fn set_smime_trust(
        &mut self,
        fp: CertFingerprint,
        trust: KeyTrust,
    ) -> Result<(), StoreError> {
        let cert = self
            .smime_certs
            .get_mut(&fp)
            .ok_or(StoreError::NoSmimeCert(fp))?;
        cert.trust = trust;
        Ok(())
    }
}
