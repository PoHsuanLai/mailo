//! The provider files mailo knows addresses by.
//!
//! Which provider an address belongs to is porter's question, answered by `matching` in a
//! provider file (`porter_provider::ProviderSet::claiming`) and, for a domain no file lists, by
//! discovery (`porter_discover`). The files are porter's, compiled into `porter-provider` at the
//! rev `Cargo.toml` pins (`porter_provider::shipped_specs`), so the pin is their one source: mailo
//! keeps no copy. mailo's own files, in `own-providers/`, are for what porter's files do not do for
//! mail: Gmail (`google.toml`, which replaces porter's file of that id, whose mail row is for a
//! person's own client and has no SMTP). They are laid over porter's like a user's files.

use porter_provider::{ProviderSet, ProviderSpec, parse_provider, shipped_specs};
use std::sync::OnceLock;

/// mailo's own files.
const OWN: &[&str] = &[include_str!("../../own-providers/google.toml")];

/// Every provider mailo knows, porter's with mailo's laid over them.
pub fn set() -> &'static ProviderSet {
    static SET: OnceLock<ProviderSet> = OnceLock::new();
    SET.get_or_init(|| ProviderSet::layered(shipped_specs(), own()))
}

/// mailo's files that parse. Every one does (a test says so); one that did not would be a
/// provider that claims nothing.
fn own() -> Vec<ProviderSpec> {
    OWN.iter()
        .filter_map(|text| parse_provider(text).ok())
        .collect()
}

#[cfg(test)]
mod tests;
