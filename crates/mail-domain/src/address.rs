//! A mailbox address: who, and where to write to them.

use crate::error::ParseAddressError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A mailbox address with its optional display name.
///
/// [`Display`](fmt::Display) writes it the way a recipient box shows it, `Ada <ada@example.test>`
/// or the bare address when there is no name, and [`FromStr`] reads one such entry back. A
/// [`parse_list`](Address::parse_list) reads a whole box of them.
///
/// ```
/// use mail_domain::Address;
///
/// let ada: Address = "\"Lovelace, Ada\" <ada@example.test>".parse().unwrap();
/// assert_eq!(ada.name.as_deref(), Some("Lovelace, Ada"));
/// assert_eq!(ada.email, "ada@example.test");
/// // A name with a comma is written quoted, so the box reads back as one recipient.
/// assert_eq!(ada.to_string(), "\"Lovelace, Ada\" <ada@example.test>");
///
/// let both = Address::parse_list("ada@example.test; Bob <bob@example.test>").unwrap();
/// assert_eq!(Address::join(&both), "ada@example.test, Bob <bob@example.test>");
///
/// assert!("not an address".parse::<Address>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    /// A bare address with no display name.
    pub fn new(email: impl Into<String>) -> Self {
        Address {
            name: None,
            email: email.into(),
        }
    }

    /// An address with a display name.
    pub fn named(name: impl Into<String>, email: impl Into<String>) -> Self {
        Address {
            name: Some(name.into()),
            email: email.into(),
        }
    }

    /// The display name when it says anything: trimmed, and `None` when blank.
    pub fn display_name(&self) -> Option<&str> {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// `text` split at its last angle brackets: what comes before them, and what is inside.
    /// `None` when there is no `<` followed by a `>`.
    ///
    /// Only the shape is looked at, so this is for the callers whose input is deliberately read
    /// leniently (a `From:` value in an old mbox, a key's user id): they judge the inside
    /// themselves. [`FromStr`] reads the same shape and then judges it strictly.
    pub fn split_angled(text: &str) -> Option<(&str, &str)> {
        let (open, close) = (text.rfind('<')?, text.rfind('>')?);
        (open < close).then(|| (&text[..open], &text[open + 1..close]))
    }

    /// The address the way an OpenPGP user id spells it: `Name <a@b.test>`, or `<a@b.test>`
    /// with no name. The name is written as it is, not quoted: a user id is one string, not an
    /// entry in a list that a comma could cut.
    pub fn user_id(&self) -> String {
        match self.display_name() {
            Some(name) => format!("{name} <{}>", self.email),
            None => format!("<{}>", self.email),
        }
    }

    /// Several addresses as one line, the way a recipient box holds them.
    pub fn join(list: &[Address]) -> String {
        list.iter()
            .map(Address::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A recipient box read into addresses.
    ///
    /// Accepts `a@b.test`, `Name <a@b.test>` and `"Name" <a@b.test>`, separated by commas or
    /// semicolons. Empty entries are skipped, because a trailing comma is what typing looks like.
    ///
    /// An entry with no `@` is an error rather than a guess. The alternative, dropping it or
    /// appending a default domain, means the person sees their recipient vanish or sends to
    /// someone they did not name. Both are silent; an error is not. One bad entry fails the whole
    /// box, because sending to everyone that could be parsed is the failure this prevents.
    ///
    /// A mailbox written twice is kept once, under its first spelling and name. Two `RCPT TO`
    /// lines for one mailbox are two deliveries.
    pub fn parse_list(input: &str) -> Result<Vec<Address>, ParseAddressError> {
        let mut out: Vec<Address> = Vec::new();
        for entry in split_entries(input) {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let address: Address = entry.parse()?;
            if !out
                .iter()
                .any(|kept| kept.email.eq_ignore_ascii_case(&address.email))
            {
                out.push(address);
            }
        }
        Ok(out)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.display_name() {
            // The separators the box splits on would otherwise cut the name in two.
            Some(name) if name.contains([',', ';']) && !name.contains('"') => {
                write!(f, "\"{name}\" <{}>", self.email)
            }
            Some(name) => write!(f, "{name} <{}>", self.email),
            None => f.write_str(&self.email),
        }
    }
}

impl FromStr for Address {
    type Err = ParseAddressError;

    /// One entry of a recipient box: `a@b.test`, `Name <a@b.test>` or `"Name" <a@b.test>`.
    ///
    /// One `@`, with something either side and no whitespace. Not a full RFC 5322 validation:
    /// that accepts things no mail server does, and rejecting what a person's own server accepts
    /// is worse than letting the server answer. This catches the mistake people actually make,
    /// which is a missing `@`.
    fn from_str(entry: &str) -> Result<Self, Self::Err> {
        let entry = entry.trim();
        let (name, email) = match Address::split_angled(entry) {
            Some((before, inside)) => {
                let name = before.trim().trim_matches('"').trim();
                ((!name.is_empty()).then(|| name.to_owned()), inside.trim())
            }
            None => (None, entry),
        };
        if email.is_empty() {
            return Err(ParseAddressError::Empty(entry.to_owned()));
        }
        let mut halves = email.split('@');
        let (local, domain) = (halves.next().unwrap_or(""), halves.next().unwrap_or(""));
        if local.is_empty() || domain.is_empty() || halves.next().is_some() {
            return Err(ParseAddressError::NotAnAddress(email.to_owned()));
        }
        if email.contains(char::is_whitespace) {
            return Err(ParseAddressError::Whitespace(email.to_owned()));
        }
        Ok(Address {
            name,
            email: email.to_owned(),
        })
    }
}

/// Split a recipient box on separators that are not inside a display name or an address.
///
/// Not `split([',', ';'])`. `"Lovelace, Ada" <ada@example.test>` is one recipient, and every
/// other mail client writes a display name containing a comma exactly that way, so a naive split
/// turns a pasted recipient into two, one of which is not an address at all.
fn split_entries(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut angled) = (0usize, false, false);
    for (i, ch) in input.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            // An unbalanced '<' would otherwise swallow the rest of the box; '>' only closes
            // what a '<' opened, so a stray '>' cannot re-enable splitting that was never off.
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' | ';' if !quoted && !angled => {
                out.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_display_writes_parses_back() {
        for address in [
            Address::new("bob@example.test"),
            Address::named("Ada Lovelace", "ada@example.test"),
            Address::named("Lovelace, Ada", "ada@example.test"),
        ] {
            assert_eq!(
                address.to_string().parse::<Address>().as_ref(),
                Ok(&address),
                "{address:?}"
            );
        }
    }

    #[test]
    fn a_blank_name_is_no_name() {
        assert_eq!(Address::named("  ", "a@b.test").to_string(), "a@b.test");
    }

    #[test]
    fn each_mistake_is_its_own_error() {
        assert_eq!(
            "Ada <>".parse::<Address>(),
            Err(ParseAddressError::Empty("Ada <>".to_owned()))
        );
        assert_eq!(
            "ada".parse::<Address>(),
            Err(ParseAddressError::NotAnAddress("ada".to_owned()))
        );
        assert_eq!(
            "a b@c.test".parse::<Address>(),
            Err(ParseAddressError::Whitespace("a b@c.test".to_owned()))
        );
    }

    #[test]
    fn the_angle_brackets_are_split_without_judging_what_is_inside() {
        assert_eq!(
            Address::split_angled("Ada <ada@example.test>"),
            Some(("Ada ", "ada@example.test"))
        );
        // The last pair, as the lenient readers of old mail and key user ids always took it.
        assert_eq!(Address::split_angled("a <b> <c>"), Some(("a <b> ", "c")));
        assert_eq!(Address::split_angled("<>"), Some(("", "")));
        for none in ["ada@example.test", "Ada <ada@example.test", "> <", ""] {
            assert_eq!(Address::split_angled(none), None, "{none:?}");
        }
    }

    #[test]
    fn a_user_id_is_the_name_then_the_angled_address() {
        assert_eq!(
            Address::named(" Ada ", "ada@example.test").user_id(),
            "Ada <ada@example.test>"
        );
        assert_eq!(
            Address::named("Lovelace, Ada", "ada@example.test").user_id(),
            "Lovelace, Ada <ada@example.test>"
        );
        assert_eq!(
            Address::new("ada@example.test").user_id(),
            "<ada@example.test>"
        );
        assert_eq!(
            Address::named("  ", "ada@example.test").user_id(),
            "<ada@example.test>"
        );
    }
}
