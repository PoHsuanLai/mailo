//! Spaces written before theme and motion were theirs.

use super::Spaces;
use crate::appearance::Legacy;
use serde_json::Value;

/// Parse `bytes`, giving each Space that has no `theme` or `motion` of its own `look`'s.
///
/// Only a missing key inherits. A Space that stored a word, even one this build does not
/// know, made a choice of its own, and that word falls to the field's default as it always
/// has. Anything that is not Spaces JSON is an empty list, as before.
pub(super) fn read_with(bytes: &[u8], look: &Legacy) -> Spaces {
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return Spaces::default();
    };
    let theme = serde_json::to_value(look.theme).unwrap_or(Value::Null);
    let motion = serde_json::to_value(look.motion).unwrap_or(Value::Null);
    if let Some(list) = value.get_mut("spaces").and_then(Value::as_array_mut) {
        for space in list.iter_mut().filter_map(Value::as_object_mut) {
            space.entry("theme").or_insert_with(|| theme.clone());
            space.entry("motion").or_insert_with(|| motion.clone());
        }
    }
    serde_json::from_value(value).unwrap_or_default()
}

#[cfg(test)]
mod tests;
