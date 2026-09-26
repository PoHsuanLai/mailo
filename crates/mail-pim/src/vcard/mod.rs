//! vCard: RFC 6350 (4.0) written, and 4.0, 3.0 (RFC 2426) and 2.1 read.
//!
//! What a mail client needs from a card is who someone is and where to write to them, so those
//! are fields — names, addresses, telephone numbers, organisation, note — and everything else
//! rides along as [`ContentLine`]s and is written back as it came. A card read from a phone and
//! exported again keeps its birthday and its postal address even though nothing here reads them.
//!
//! A card may stand for a group rather than a person (`KIND:group`, RFC 6350 §6.1.4), and then
//! lists who is in it (`MEMBER`, §6.6.5): each member a URI, usually a `urn:uuid:` naming another
//! card by its `UID` or a `mailto:` naming an address. [`Member::of`] says which. A member that
//! names nothing this program knows is still a member, and is written back as it was read.

mod member;
mod read;
mod write;

pub use member::{Member, uid_key};

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
    /// `KIND`: what the card stands for. `None` when the card does not say, which RFC 6350
    /// §6.1.4 reads as an individual.
    pub kind: Option<CardKind>,
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
    /// `MEMBER` URIs of a group, in the order written, each as the card wrote it. Empty for a
    /// card that is not a group: §6.6.5 allows the property only with `KIND:group`, so on any
    /// other card it is kept among [`Card::other`] and written back where it was.
    pub members: Vec<String>,
    /// Every other property, as read, to be written back unchanged.
    pub other: Vec<ContentLine>,
}

/// `KIND` (RFC 6350 §6.1.4): what a card stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardKind {
    Individual,
    /// A group of people or resources, whose members are its `MEMBER`s.
    Group,
    Org,
    Location,
    /// An `x-name` or a kind registered after RFC 6350, lower-cased, written back as read.
    Other(String),
}

impl CardKind {
    /// The kind a `KIND` value names, compared without regard to case as §6.1.4 asks.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "individual" => Self::Individual,
            "group" => Self::Group,
            "org" => Self::Org,
            "location" => Self::Location,
            other => Self::Other(other.to_owned()),
        }
    }

    /// The value as written.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Individual => "individual",
            Self::Group => "group",
            Self::Org => "org",
            Self::Location => "location",
            Self::Other(other) => other,
        }
    }
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
            kind: None,
            formatted_name: None,
            name: None,
            emails: Vec::new(),
            phones: Vec::new(),
            org: Vec::new(),
            note: None,
            revision: None,
            photo: None,
            members: Vec::new(),
            other: Vec::new(),
        }
    }

    /// A 4.0 group named `name` with `members`, each a `MEMBER` URI.
    pub fn group(name: &str, members: Vec<String>) -> Self {
        Self {
            kind: Some(CardKind::Group),
            formatted_name: Some(name.to_owned()),
            members,
            ..Self::new()
        }
    }

    /// Whether the card stands for a group.
    pub fn is_group(&self) -> bool {
        self.kind == Some(CardKind::Group)
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
