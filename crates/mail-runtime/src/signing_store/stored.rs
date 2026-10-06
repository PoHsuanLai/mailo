//! What one keyring entry holds, as it is written.
//!
//! Frozen (the account half is read once, by `adopt.rs`, and then never again): entries written by every earlier build must keep reading,
//! so the shape here is the shape of `tests/fixtures/keyring/`, whatever types the rest of mailo
//! holds a secret in. One entry holds one of four things, and which is decided by the entry's
//! name (`<account>:oauth`, `openpgp:<fingerprint>`), so the same JSON serves an account's
//! credential and a signing key.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub(crate) enum Stored {
    Password(String),
    #[serde(rename = "oauth")]
    OAuth {
        access: String,
        refresh: String,
        expires_at: DateTime<Utc>,
    },
    #[serde(rename = "openpgp")]
    OpenPgp(String),
    #[serde(rename = "smime_key")]
    SmimeKey(String),
}

// Written by hand, like every secret's: a derived Debug would print it.
impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stored(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries every earlier build wrote. They are never regenerated: if one stops decoding, or
    /// decodes to something that writes back differently, a person's saved sign-ins and keys
    /// are gone.
    const FROZEN: &[(&str, &str)] = &[
        (
            "account.json",
            include_str!("../../tests/fixtures/keyring/account.json"),
        ),
        (
            "openpgp.json",
            include_str!("../../tests/fixtures/keyring/openpgp.json"),
        ),
        (
            "smime.json",
            include_str!("../../tests/fixtures/keyring/smime.json"),
        ),
    ];

    #[test]
    fn every_frozen_entry_still_reads_and_writes_back_unchanged() {
        for (name, text) in FROZEN {
            let before: serde_json::Value = serde_json::from_str(text).expect(name);
            let entries: Vec<Stored> = serde_json::from_str(text).expect(name);
            assert!(!entries.is_empty(), "{name}");
            let after = serde_json::to_value(&entries).expect(name);
            assert_eq!(before, after, "{name}");
        }
    }
}
