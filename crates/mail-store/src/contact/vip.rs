//! VIPs: the people whose mail matters most, marked by hand (migration 0043).
//!
//! A mark on a person, keyed by the address the book keys them by ([`super::normalise`]), and
//! one list across every account and Space. It is not a column of the book: a VIP can be marked
//! before any mail has taught the book their address, and a contact forgotten and learnt again
//! is still a VIP.

use serde::{Deserialize, Serialize};

/// One VIP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vip {
    /// Lower-cased and trimmed. The key.
    pub address: String,
    /// The name they were marked with, for a list to show when the book has none.
    pub name: Option<String>,
}

/// The order VIPs are listed in: by address, which is the key and never changes.
pub(crate) fn listed(mut vips: Vec<Vip>) -> Vec<Vip> {
    vips.sort_by(|a, b| a.address.cmp(&b.address));
    vips
}
