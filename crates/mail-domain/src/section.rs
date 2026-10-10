//! An IMAP body section: the name a part of a message is fetched by.

use crate::error::ParseSectionError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A body section specifier as RFC 3501 section 6.4.5 spells it: `"1"`, `"1.3"`, `"2.MIME"`,
/// `"HEADER"`, `"TEXT"`, or the empty root of a multipart message.
///
/// Sections come from a server and are written into command lines, so one is checked once, where
/// it enters, and nothing that holds a `Section` has to check it again. It is stored and sent as
/// the text it is: serde and SQL see the same string they always did.
///
/// The part numbers are positive and at most nine digits; a trailing `MIME`, `HEADER` or `TEXT`
/// is only valid after a part number, and `HEADER` and `TEXT` alone name the whole message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Section(String);

impl Section {
    /// The root of a multipart message, which has no number of its own.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// The headers of the whole message.
    pub fn header() -> Self {
        Self("HEADER".to_owned())
    }

    /// The part at this path of 1-based indexes, `[1, 3]` being `"1.3"`. `None` for an empty
    /// path or an index of zero, which no section has.
    pub fn of(path: &[u32]) -> Option<Self> {
        if path.is_empty() || path.contains(&0) {
            return None;
        }
        let text = path
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        text.parse().ok()
    }

    /// The headers of this part, `"2"` becoming `"2.MIME"`. `None` for the root and for a
    /// section that already names a header or text, which have none of their own.
    pub fn mime(&self) -> Option<Self> {
        let numbers = !self.0.is_empty() && self.0.bytes().all(|b| b == b'.' || b.is_ascii_digit());
        numbers.then(|| Self(format!("{}.MIME", self.0)))
    }

    /// Whether this is the root, which names nothing that can be fetched.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid(text: &str) -> bool {
    if text.is_empty() || text == "HEADER" || text == "TEXT" {
        return true;
    }
    let numbers = ["MIME", "HEADER", "TEXT"]
        .iter()
        .find_map(|word| {
            text.strip_suffix(word)
                .and_then(|rest| rest.strip_suffix('.'))
        })
        .unwrap_or(text);
    numbers
        .split('.')
        .all(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) && n != "0")
}

impl FromStr for Section {
    type Err = ParseSectionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if is_valid(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ParseSectionError(text.to_owned()))
        }
    }
}

impl TryFrom<String> for Section {
    type Error = ParseSectionError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if is_valid(&text) {
            Ok(Self(text))
        } else {
            Err(ParseSectionError(text))
        }
    }
}

impl From<Section> for String {
    fn from(section: Section) -> String {
        section.0
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Section {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Section {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Section {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}
