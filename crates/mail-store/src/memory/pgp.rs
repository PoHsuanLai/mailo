//! OpenPGP keys and Autocrypt peers in the in-memory store. Kept in step with `sqlite/pgp.rs`,
//! which the parity tests in `tests/pgp_keys.rs` hold it to.

use super::Inner;
use crate::StoreError;
use mail_domain::{AutocryptPeer, Fingerprint, KeyId, KeyTrust, PgpKey};

impl Inner {
    fn pgp_keys_where(&self, keep: impl Fn(&PgpKey) -> bool) -> Vec<PgpKey> {
        crate::pgp::preferred(
            self.pgp_keys
                .values()
                .filter(|k| keep(k))
                .cloned()
                .collect(),
        )
    }

    pub(super) fn all_pgp_keys(&self) -> Vec<PgpKey> {
        // By fingerprint, as the SQLite side sorts them.
        self.pgp_keys.values().cloned().collect()
    }

    pub(super) fn pgp_keys_for(&self, address: &str) -> Vec<PgpKey> {
        let address = address.trim().to_ascii_lowercase();
        // `json_each(emails) WHERE value = ?`: an exact match on the stored, lower-cased text.
        self.pgp_keys_where(|k| k.emails.contains(&address))
    }

    pub(super) fn pgp_keys_by_id(&self, id: KeyId) -> Vec<PgpKey> {
        self.pgp_keys_where(|k| k.key_ids.contains(&id))
    }

    pub(super) fn put_pgp_key(&mut self, key: PgpKey) -> PgpKey {
        let merged = match self.pgp_keys.remove(&key.fingerprint) {
            Some(held) => held.merged(key),
            None => key,
        };
        self.pgp_keys.insert(merged.fingerprint, merged.clone());
        merged
    }

    pub(super) fn set_pgp_trust(
        &mut self,
        fp: Fingerprint,
        trust: KeyTrust,
    ) -> Result<(), StoreError> {
        let key = self.pgp_keys.get_mut(&fp).ok_or(StoreError::NoPgpKey(fp))?;
        key.trust = trust;
        Ok(())
    }

    pub(super) fn autocrypt_peer(&self, address: &str) -> Option<AutocryptPeer> {
        self.autocrypt
            .get(&address.trim().to_ascii_lowercase())
            .cloned()
    }

    pub(super) fn put_autocrypt_peer(&mut self, peer: &AutocryptPeer) {
        let address = peer.address.trim().to_ascii_lowercase();
        self.autocrypt.insert(
            address.clone(),
            AutocryptPeer {
                address,
                ..peer.clone()
            },
        );
    }
}
