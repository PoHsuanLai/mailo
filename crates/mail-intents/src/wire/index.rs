//! What mailo hands the router's index: one `org.quire.Intents1.Index.Push` batch.
//!
//! The router keeps what each app pushes and searches it as the person types, without asking the
//! app (`docket-core`'s `IndexBatch` and `IndexEntry`). Its keys and units are bare strings and
//! numbers; which Spaces may find an entry is adjacently tagged, like the rest of the wire.

use mail_core::findable::{Findable, Kind};
use serde::Serialize;

/// One push: what is new or changed, and what is gone, in the epoch the last `Reset` started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexBatch {
    /// The epoch.
    pub epoch: u64,
    /// What is new or changed.
    pub upserts: Vec<IndexEntry>,
    /// The keys of what is gone, of any kind.
    pub removals: Vec<String>,
}

/// One thing the router may find. Its words are plain text: the router labels them from the
/// trust the manifest declares for the kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexEntry {
    /// The key the kind's intents name it by.
    pub key: String,
    /// `mail.thread` or `mail.contact`.
    pub kind: String,
    /// Its title.
    pub title: String,
    /// Its subtitle.
    pub subtitle: String,
    /// More words that find it.
    pub keywords: Vec<String>,
    /// When it last changed, in seconds since the Unix epoch.
    pub updated: i64,
    /// Which Spaces may find it.
    pub space: SpaceScope,
}

/// Which of the desktop's Spaces may find an entry. Mail's Spaces are views inside its window,
/// not the desktop's, so every entry is found in all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SpaceScope {
    /// Every Space, and outside them.
    Any,
}

/// The kind the manifest names a findable thing's kind by, whose intents key it the same way.
pub fn kind_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Conversation => "mail.thread",
        Kind::Contact => "mail.contact",
    }
}

impl IndexEntry {
    /// The entry for `found`.
    pub fn of(found: &Findable) -> IndexEntry {
        IndexEntry {
            key: found.key.clone(),
            kind: kind_of(found.kind).to_owned(),
            title: found.title.clone(),
            subtitle: found.subtitle.clone(),
            keywords: found.keywords.clone(),
            updated: found.updated.timestamp(),
            space: SpaceScope::Any,
        }
    }
}
