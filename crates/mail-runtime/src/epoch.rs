//! A count of the changes to the keys and certificates this process holds or trusts: OpenPGP
//! keys and S/MIME certificates alike, one counter for both.
//!
//! Whatever caches the result of opening a message — the reader's decrypted text, a badge —
//! keeps the count it was made under, and asks again when the count has moved: a key imported,
//! made, deleted or trusted, or one learnt from arriving mail, may change what a message opens
//! to. Only a change counts: seeing a key again that is already held, byte for byte, does not
//! move it, so a sync does not throw away every decrypted message it did not touch.
//!
//! In this process only. Another process writing the same store (a second `mailo`) is not seen;
//! its changes are picked up when the cache is next rebuilt.

use std::sync::atomic::{AtomicU64, Ordering};

static EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current count.
pub fn keys() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

/// Record a change.
pub fn keys_changed() {
    EPOCH.fetch_add(1, Ordering::Relaxed);
}
