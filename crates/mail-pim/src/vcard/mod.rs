//! vCard: RFC 6350 (4.0) written, and 4.0, 3.0 (RFC 2426) and 2.1 read.
//!
//! What a mail client needs from a card is who someone is and where to write to them, so those
//! are fields — names, addresses, telephone numbers, organisation, note — and everything else
//! rides along as [`ContentLine`]s and is written back as it came. A card read from a phone and
//! exported again keeps its birthday and its postal address even though nothing here reads them.

mod read;
mod write;

pub use read::{parse, parse_bytes};
pub use write::{write, write_all};

use crate::line::ContentLine;

/// The version a card was read as. Every card is written as 4.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V2_1,
    V3_0,
    V4_0,
}

/// One contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub version: Version,
    /// `UID`: what a CardDAV server and a sync partner know this card by.
    pub uid: Option<String>,
    /// `FN`, the name as it is displayed.
    pub formatted_name: Option<String>,
    /// `N`, the name in parts.
    pub name: Option<Name>,
    /// `EMAIL`, in the order written.
    pub emails: Vec<Email>,
    /// `TEL`, in the order written.
    pub phones: Vec<Phone>,
    /// `ORG`: the organisation, then its units.
    pub org: Vec<String>,
    pub note: Option<String>,
    /// `REV`, as written: a timestamp in one of several forms, compared by nothing here.
    pub revision: Option<String>,
    /// `PHOTO`, as a URI: a 4.0 card's own, or a `data:` URI built from an older card's inline
    /// base64 so it can be written as 4.0.
    pub photo: Option<String>,
    /// Every other property, as read, to be written back unchanged.
    pub other: Vec<ContentLine>,
}

/// `N`: family; given; additional; prefixes; suffixes. Each may hold several values, which are
/// kept comma-joined as the card wrote them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Name {
    pub family: String,
    pub given: String,
    pub additional: String,
    pub prefixes: String,
    pub suffixes: String,
}

/// One `EMAIL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub address: String,
    /// `TYPE` values, lower-cased: `work`, `home`, and in older cards `internet`.
    pub kinds: Vec<String>,
    /// `PREF`, 1 (most preferred) to 100. A 3.0 `TYPE=pref` is read as 1.
    pub pref: Option<u8>,
}

/// One `TEL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    /// The number as written, less a 4.0 `tel:` scheme.
    pub number: String,
    /// `TYPE` values, lower-cased: `cell`, `work`, `voice`…
    pub kinds: Vec<String>,
    pub pref: Option<u8>,
}

impl Card {
    /// An empty 4.0 card.
    pub fn new() -> Self {
        Self {
            version: Version::V4_0,
            uid: None,
            formatted_name: None,
            name: None,
            emails: Vec::new(),
            phones: Vec::new(),
            org: Vec::new(),
            note: None,
            revision: None,
            photo: None,
            other: Vec::new(),
        }
    }

    /// What to call this person: `FN`, else the given and family names, else nothing.
    pub fn display_name(&self) -> Option<String> {
        if let Some(name) = self.formatted_name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            return Some(name.to_owned());
        }
        let n = self.name.as_ref()?;
        let joined = [n.given.as_str(), n.family.as_str()]
            .iter()
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        (!joined.is_empty()).then_some(joined)
    }

    /// The addresses, most preferred first; equal preferences keep the card's order.
    pub fn addresses_by_preference(&self) -> Vec<&str> {
        let mut emails: Vec<&Email> = self.emails.iter().collect();
        emails.sort_by_key(|e| e.pref.unwrap_or(u8::MAX));
        emails.iter().map(|e| e.address.as_str()).collect()
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}
