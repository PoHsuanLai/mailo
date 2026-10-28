//! VIPs in the in-memory store. Kept in step with `sqlite/vips.rs`; the parity tests in
//! `tests/store/vips.rs` hold the two to each other.

use super::Inner;
use crate::StoreError;
use crate::contact::learn;
use crate::contact::vip::{Vip, listed};

impl Inner {
    pub(super) fn every_vip(&self) -> Vec<Vip> {
        listed(
            self.vips
                .iter()
                .map(|(address, name)| Vip {
                    address: address.clone(),
                    name: name.clone(),
                })
                .collect(),
        )
    }

    pub(super) fn mark_vip(
        &mut self,
        address: &str,
        name: Option<&str>,
    ) -> Result<Vip, StoreError> {
        let address =
            learn::normalise(address).ok_or_else(|| StoreError::BadAddress(address.to_owned()))?;
        let kept = self.vips.entry(address.clone()).or_insert(None);
        if let Some(name) = learn::clean_name(name) {
            *kept = Some(name);
        }
        Ok(Vip {
            address,
            name: kept.clone(),
        })
    }

    pub(super) fn unmark_vip(&mut self, address: &str) -> bool {
        learn::normalise(address).is_some_and(|address| self.vips.remove(&address).is_some())
    }
}
