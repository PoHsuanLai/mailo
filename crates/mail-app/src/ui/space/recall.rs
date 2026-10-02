//! Where a Space was left.

use mail_domain::{AccountId, ThreadId};
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The place, open thread and account tile a Space showed when you left it.
///
/// The place is stored by name, not position: a label added while you were elsewhere moves
/// every label after it, and "the third place" would then be a different place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recall {
    /// The place's name, as the sidebar shows it. Empty is the first place.
    pub place: String,
    /// The thread open in the reader, if any.
    pub open: Option<ThreadId>,
    /// The account tile that was pressed. `None` is every account in the Space.
    pub account: Option<AccountId>,
}

/// The stored map, or an empty one when it is not a map of recalls.
///
/// Where you were is worth less than the Spaces stored beside it: a damaged entry must not
/// fail the whole file, so the map is dropped on its own.
pub(super) fn de_recall<'de, D>(deserializer: D) -> Result<BTreeMap<usize, Recall>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

#[cfg(test)]
mod tests;
