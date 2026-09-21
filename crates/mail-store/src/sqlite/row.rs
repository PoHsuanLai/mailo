//! Conversions between SQLite rows and domain values.
//!
//! Several columns hold JSON in the serde form fixed by `CONVENTIONS.md` section 3. Decoding
//! one that no longer matches its type is [`StoreError::Decode`] — almost always a missing
//! migration or a `#[serde(default)]` that was not added when a field was.

use crate::StoreError;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Decode a JSON column.
pub fn json<T: DeserializeOwned>(what: &str, text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError::Decode {
        what: what.to_owned(),
        why: e.to_string(),
    })
}

/// Encode a value for a JSON column.
pub fn to_json<T: Serialize>(what: &str, value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError::Decode {
        what: what.to_owned(),
        why: e.to_string(),
    })
}

/// Decode a UUID-shaped id column.
pub fn uuid(what: &str, text: &str) -> Result<Uuid, StoreError> {
    text.parse().map_err(|e: uuid::Error| StoreError::Decode {
        what: what.to_owned(),
        why: e.to_string(),
    })
}

/// Decode an RFC 3339 timestamp column.
///
/// Stored in RFC 3339 UTC so the text sorts in the same order as the instants, which is what
/// lets the keyset pagination index work without parsing every row.
pub fn time(what: &str, text: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| StoreError::Decode {
            what: what.to_owned(),
            why: e.to_string(),
        })
}

/// Encode a timestamp for storage.
pub fn from_time(at: DateTime<Utc>) -> String {
    // Nanoseconds, and always nine digits. Fixed width is what makes the text sort in the same
    // order as the instants, which is what the keyset pagination index relies on — chrono's
    // serde form is variable width, where `...06.001Z` sorts BEFORE `...06Z`.
    //
    // Nine digits rather than three because a `DateRange` boundary inside the same millisecond
    // would otherwise compare equal here while `Filter::fit` compares the real instants, and
    // the two would disagree.
    at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}
