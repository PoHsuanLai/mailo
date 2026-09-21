//! Closed vocabulary for mail state. No `bool` fields: a named variant documents meaning at
//! the use site and makes the illegal combinations unrepresentable.

use serde::{Deserialize, Serialize, Serializer, de::Deserializer};

/// The six special-use roles. Deliberately closed: any other server folder becomes a
/// [`crate::LabelOrigin::Provider`] label rather than a new role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Trash,
    Spam,
}

impl MailboxRole {
    /// Every role, in display order.
    pub const ALL: [MailboxRole; 6] = [
        MailboxRole::Inbox,
        MailboxRole::Archive,
        MailboxRole::Sent,
        MailboxRole::Drafts,
        MailboxRole::Trash,
        MailboxRole::Spam,
    ];

    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// The set of roles a thread spans.
///
/// A thread is not in one place: on Gmail a conversation you replied to has messages in
/// `Inbox` *and* `Sent` at the same time, and collapsing that to a single role files the
/// thread in the wrong list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct MailboxSet(u8);

impl MailboxSet {
    /// The empty set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// A set holding exactly one role.
    pub const fn only(role: MailboxRole) -> Self {
        Self(role.bit())
    }

    /// Whether `role` is present.
    pub const fn contains(self, role: MailboxRole) -> bool {
        self.0 & role.bit() != 0
    }

    /// This set with `role` added.
    pub const fn with(self, role: MailboxRole) -> Self {
        Self(self.0 | role.bit())
    }

    /// This set with `role` removed.
    pub const fn without(self, role: MailboxRole) -> Self {
        Self(self.0 & !role.bit())
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether no role is present.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The roles present, in [`MailboxRole::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = MailboxRole> {
        MailboxRole::ALL
            .into_iter()
            .filter(move |r| self.contains(*r))
    }
}

impl FromIterator<MailboxRole> for MailboxSet {
    fn from_iter<I: IntoIterator<Item = MailboxRole>>(iter: I) -> Self {
        iter.into_iter().fold(Self::empty(), Self::with)
    }
}

// Persisted as a list of role names, never as the bitmask. The bit positions are an
// implementation detail; a magic number in SQLite would be a schema we could not read.
impl Serialize for MailboxSet {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.iter().collect::<Vec<_>>().serialize(s)
    }
}

impl<'de> Deserialize<'de> for MailboxSet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Vec::<MailboxRole>::deserialize(d)?.into_iter().collect())
    }
}

/// Whether a message has been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadState {
    #[default]
    Unread,
    Read,
}

/// Whether a message is starred (`\Flagged` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Star {
    #[default]
    Unstarred,
    Starred,
}

/// Which way a label assignment moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Membership {
    In,
    Out,
}

impl Membership {
    /// The opposite membership.
    pub const fn flip(self) -> Self {
        match self {
            Membership::In => Membership::Out,
            Membership::Out => Membership::In,
        }
    }
}

/// Attachment presence. `Present { count: 0 }` is unrepresentable by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Attachments {
    #[default]
    None,
    Present {
        count: u32,
    },
}

impl Attachments {
    /// Presence for a given count, collapsing zero to [`Attachments::None`].
    pub const fn of(count: u32) -> Self {
        if count == 0 {
            Attachments::None
        } else {
            Attachments::Present { count }
        }
    }
}

/// Manual ordering within a list. `Rank` is sparse so a reorder rewrites one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Pin {
    #[default]
    Unpinned,
    Rank(i64),
}

/// Whether a thread is hidden until a future instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Snooze {
    #[default]
    Inactive,
    Until(chrono::DateTime<chrono::Utc>),
}

/// Where a label came from, which decides whether the user may rename or delete it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelOrigin {
    /// Created here by the user.
    User,
    /// Mirrors something the server owns: an IMAP folder, a Gmail category.
    Provider,
}

/// Whether an identity is the one used unless the user picks otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsDefault {
    Default,
    Alternate,
}

/// Whether a list groups messages into conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Threading {
    #[default]
    Threaded,
    Single,
}
