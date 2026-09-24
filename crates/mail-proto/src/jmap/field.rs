//! Reading fields out of a stranger's JSON without trusting its shape.
//!
//! Every accessor names what it was looking for, so a server that sends a number where a string
//! belongs produces an error that says which field, not a bare "invalid type".

use crate::ProtoError;
use serde_json::{Map, Value};

pub(super) fn malformed(what: impl Into<String>) -> ProtoError {
    ProtoError::Malformed(format!("JMAP: {}", what.into()))
}

/// `value` as an object, or an error naming `what` it should have been.
pub(super) fn object<'a>(
    value: &'a Value,
    what: &str,
) -> Result<&'a Map<String, Value>, ProtoError> {
    value
        .as_object()
        .ok_or_else(|| malformed(format!("{what} is not an object")))
}

/// A required string field.
pub(super) fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ProtoError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(format!("`{key}` is missing or not a string")))
}

/// An optional string field. `null` and absence are both `None`; any other type is an error.
pub(super) fn opt_string<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, ProtoError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(malformed(format!("`{key}` is not a string"))),
    }
}

/// A list of strings. `null` and absence are the empty list, which is what RFC 8621 means by
/// `null` for `messageId`, `inReplyTo` and `references`.
pub(super) fn strings(value: &Value, key: &str) -> Result<Vec<String>, ProtoError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| malformed(format!("`{key}` holds something not a string")))
            })
            .collect(),
        Some(_) => Err(malformed(format!("`{key}` is not a list"))),
    }
}

/// The keys of a `String[Boolean]` map whose value is `true`: `mailboxIds`, `keywords`.
///
/// RFC 8621 says the values are always `true`; a `false` is read as absence rather than
/// trusted, since an entry that says "not in this mailbox" means the email is not in it.
pub(super) fn true_keys(value: &Value, key: &str) -> Result<Vec<String>, ProtoError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map
            .iter()
            .filter(|(_, v)| v.as_bool() == Some(true))
            .map(|(k, _)| k.clone())
            .collect()),
        Some(_) => Err(malformed(format!("`{key}` is not a map"))),
    }
}

/// An unsigned number, or `default` when absent.
pub(super) fn unsigned(value: &Value, key: &str, default: u64) -> Result<u64, ProtoError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| malformed(format!("`{key}` is not an unsigned number"))),
    }
}

/// An RFC 3339 date, or `None` when absent or unreadable.
///
/// Unreadable is `None` rather than an error because every date here has a fallback — the
/// `Date` header, or the moment of the sync — and one server's odd timestamp must not stop a
/// thousand emails arriving.
pub(super) fn date(value: &Value, key: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let text = value.get(key)?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}
