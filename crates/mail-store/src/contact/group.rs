//! Contact groups: a name for several people at once, which the composer expands into its
//! members.
//!
//! A group is a vCard `KIND:group` (RFC 6350 §6.1.4) and its members are that card's `MEMBER`
//! URIs (§6.6.5), kept exactly as written: a `mailto:` naming an address, a `urn:uuid:` naming
//! another card by its `UID`, or anything else a server or an export put there. Who a URI names
//! is decided when the group is used, not when it is stored, so a member naming a card this
//! client has not seen (yet, or ever) is kept, and written back, rather than dropped.

use serde::{Deserialize, Serialize};

/// What a group is known by here.
///
/// A group synced from an address book is known by its card's URL, which is unique across
/// every book; one made or imported here by `local:` and its `UID`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupId(pub String);

impl GroupId {
    /// The id of a group made here whose card's `UID` is `uid`.
    pub fn local(uid: &str) -> Self {
        Self(format!("local:{uid}"))
    }
}

impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    /// The card's `UID`, which another group's `MEMBER` may name it by. `None` only for a synced
    /// card that has none.
    pub uid: Option<String>,
    /// What the user calls it, and what typing in To finds it by.
    pub name: String,
    /// `MEMBER` URIs, in order. Empty is a group with nobody in it yet, which is valid.
    pub members: Vec<String>,
    pub home: GroupHome,
}

/// Where a group lives, which decides where an edit goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum GroupHome {
    /// Made or imported here. Only ever here, and in an export.
    Local,
    /// A card of a synced CardDAV address book.
    Book {
        /// The collection's URL: the [`crate::AddressBook`] it syncs with.
        url: String,
        /// The card's URL, resolved against the collection's.
        href: String,
        edit: Edit,
    },
}

/// Whether a synced group holds a change the server has not been sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edit {
    /// As the server last sent it, or as it last accepted it.
    Synced,
    /// Edited here: the next sync of its book writes it back, and until then a newer card from
    /// the server does not overwrite the edit.
    Edited,
}

impl Group {
    /// The address book a synced group belongs to, `None` for a local one.
    pub fn book(&self) -> Option<&str> {
        match &self.home {
            GroupHome::Local => None,
            GroupHome::Book { url, .. } => Some(url),
        }
    }
}
