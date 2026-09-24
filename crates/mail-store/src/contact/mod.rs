//! The address book: who the user corresponds with, learned from their mail, plus whoever they
//! typed in or synced from an address book.
//!
//! One book across every account, because the person is the same whichever account wrote to
//! them; each entry remembers which account last corresponded with them. Keyed by address,
//! lower-cased: a card with three addresses is three entries with one name.
//!
//! What feeds it:
//!
//! - every message ingested once — a message in Sent counts as written to each recipient, and
//!   any other as received from its sender (spam and other clients' drafts count as nothing);
//! - every submission the outbox confirms, as written to its recipients, with the names as the
//!   user typed them — the Sent copy of the same message, when it later arrives, is recognised
//!   and not counted a second time;
//! - [`crate::Store::put_contact`], for an entry the user typed, imported or synced.
//!
//! Autocomplete ranks by frecency: every event adds `weight × 2^(t / half-life)` to a sum, kept
//! as its base-2 logarithm, which orders exactly as a decaying score would at any later moment
//! without the query needing a clock. Writing to someone weighs [`learn::WRITTEN_WEIGHT`] times
//! receiving from them.

pub(crate) mod learn;

pub use learn::normalise;

use chrono::{DateTime, Utc};
use mail_domain::AccountId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One address and what is known about its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// Lower-cased and trimmed. The key.
    pub address: String,
    /// The name to show: one the user gave, else the latest one they wrote to this address
    /// with, else the latest one this address sent with.
    pub name: Option<String>,
    /// Messages the user sent to this address.
    pub written: Tally,
    /// Messages that arrived from this address.
    pub received: Tally,
    /// The account that corresponded with this address most recently, where one has.
    pub account: Option<AccountId>,
    pub origin: Origin,
    pub kind: Kind,
}

/// How many, and when the latest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tally {
    pub count: u32,
    pub last: Option<DateTime<Utc>>,
}

/// Where an entry came from, which decides whether mail may rename it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Origin {
    /// Learned from mail. The name follows what the mail says.
    History,
    /// Typed or imported by the user. Mail never renames it.
    Manual,
    /// Synced from an address book, named by its source (`carddav:<collection url>`). Mail never
    /// renames it; the next sync of that book may.
    Book(String),
}

/// What kind of sender an address is, which decides whether autocomplete offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Person,
    /// Mail from it came through a list or from a no-reply address. Not offered until the user
    /// writes to it.
    Bulk,
    /// The user's own address, learned from what they sent. Never offered.
    Own,
}

impl Contact {
    /// Whether autocomplete offers this entry.
    ///
    /// Everything but the user's own addresses, and bulk senders the user has never written to
    /// and never added by hand.
    pub fn offered(&self) -> bool {
        match self.kind {
            Kind::Own => false,
            Kind::Bulk => self.written.count > 0 || self.origin != Origin::History,
            Kind::Person => true,
        }
    }
}

/// What the last CardDAV sync of one address book left behind: where to resume, and which
/// addresses each card put in the book, so a card that changes or goes can take its own out.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AddressBook {
    /// The collection's URL. The key.
    pub url: String,
    /// The RFC 6578 sync token the last sync ended with. `None` before the first, and for a
    /// server that does not offer `sync-collection`, which is synced by etags instead.
    pub token: Option<String>,
    /// Every card, by its URL resolved against the collection's.
    pub cards: BTreeMap<String, BookCard>,
    /// The account the book was added under: whose sign-in it presents, or under whose name
    /// its password is kept in the keyring.
    #[serde(default)]
    pub account: Option<AccountId>,
    /// The login for a password-protected book. `None` presents the account's own sign-in.
    #[serde(default)]
    pub login: Option<String>,
}

/// One card as last synced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BookCard {
    /// Compared with the server's to tell whether the card changed; sent as `If-Match` to write.
    pub etag: String,
    /// The addresses this card put in the book, normalised.
    pub addresses: Vec<String>,
    /// The card as the server sent it.
    pub vcard: String,
}
