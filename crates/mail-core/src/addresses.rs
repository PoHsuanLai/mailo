//! A set of email addresses, compared the way the store compares them.
//!
//! One type for every "is this one of those people" question: the user's own addresses
//! ([`crate::notify::Own`]) and the VIPs ([`crate::vip::Vips`]).
//!
//! Equality is [`mail_store::contact::normalise`]'s, so a set agrees with the address book about
//! who is who: surrounding space and angle brackets are dropped, and the whole address is
//! lower-cased, Unicode included. RFC 5321 lets a server treat the local part's case as
//! significant; none that matters does, and `Ada@x` and `ada@x` are one person to the book. A
//! string that is not an address has no place in a set and is never found in one.

use mail_store::contact::normalise;

/// Addresses, normalised, sorted and once each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Addresses(Vec<String>);

impl Addresses {
    pub fn new<S: AsRef<str>>(addresses: impl IntoIterator<Item = S>) -> Self {
        let mut all: Vec<String> = addresses
            .into_iter()
            .filter_map(|address| normalise(address.as_ref()))
            .collect();
        all.sort();
        all.dedup();
        Addresses(all)
    }

    /// Whether `email` is in the set, whatever its case or its brackets.
    pub fn includes(&self, email: &str) -> bool {
        normalise(email).is_some_and(|email| self.0.binary_search(&email).is_ok())
    }

    /// The addresses, normalised and sorted.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_holds_each_address_once_as_the_book_names_it() {
        // (case, what went in, what the set holds)
        let cases: [(&str, &[&str], &[&str]); 6] = [
            (
                "case folds",
                &["Ada@Example.test", "ada@example.test"],
                &["ada@example.test"],
            ),
            (
                "sorted",
                &["b@x.test", "a@x.test"],
                &["a@x.test", "b@x.test"],
            ),
            (
                "brackets and space go",
                &[" <Ada@x.test> "],
                &["ada@x.test"],
            ),
            (
                "unicode lower-cases",
                &["Ünal@Exämple.test"],
                &["ünal@exämple.test"],
            ),
            (
                "a word that is no address is left out",
                &["ada", "", "@x.test", "a b@x.test"],
                &[],
            ),
            ("nothing in, nothing held", &[], &[]),
        ];
        for (case, put, want) in cases {
            assert_eq!(
                Addresses::new(put.iter().copied()).as_slice(),
                want,
                "{case}"
            );
        }
    }

    #[test]
    fn a_set_finds_an_address_however_it_is_written() {
        let set = Addresses::new(["Ada@Example.test", "ünal@exämple.test"]);
        // (case, asked, found)
        let cases = [
            ("same", "ada@example.test", true),
            ("upper", "ADA@EXAMPLE.TEST", true),
            ("bracketed", "<ada@example.test>", true),
            ("unicode upper", "Ünal@Exämple.test", true),
            ("another person", "bob@example.test", false),
            ("a prefix is not the person", "ada@example.tes", false),
            ("not an address", "ada", false),
            ("empty", "", false),
        ];
        for (case, asked, found) in cases {
            assert_eq!(set.includes(asked), found, "{case}");
        }
        assert!(!Addresses::default().includes("ada@example.test"));
    }
}
