//! VIPs: the people whose mail matters most, as Mail on the Mac has them.
//!
//! A mark on a person, kept by the store ([`mail_store::Store::put_vip`]) by the address the book
//! keys them by, and one list across every account and Space. A Space's pinned people are that
//! Space's shortcuts and stay so; they are offered as VIPs to mark, not made VIPs by being pinned.
//!
//! What a VIP changes is said here once, for every surface: `is:vip` finds their conversations
//! ([`Vips::filter`], through [`crate::query::parse_with`]), the VIPs place lists them, a row and
//! the reader mark their name, and "Notify for: VIPs only" announces their mail alone
//! ([`crate::notify::Whom`]).

use crate::error::{CoreError, Logged};
use mail_domain::{Filter, TextMatch};
use mail_store::Store;

pub use mail_store::Vip;

/// The VIPs' addresses, lower-cased, sorted and once each: what a filter, a mark and a
/// notification ask of the list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vips(Vec<String>);

impl Vips {
    pub fn new<S: AsRef<str>>(addresses: impl IntoIterator<Item = S>) -> Self {
        let mut all: Vec<String> = addresses
            .into_iter()
            .map(|a| a.as_ref().trim().to_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        all.sort();
        all.dedup();
        Vips(all)
    }

    /// Whether `email` is a VIP's, whatever its case.
    pub fn includes(&self, email: &str) -> bool {
        self.0.binary_search(&email.trim().to_lowercase()).is_ok()
    }

    /// The addresses, sorted.
    pub fn addresses(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// What `is:vip` finds: the conversations whose sender is one of them, matched on the whole
    /// address as `from:` matches a word of it. Nothing at all while there are none.
    pub fn filter(&self) -> Filter {
        match self.0.as_slice() {
            [] => Filter::Nothing,
            [one] => from(one),
            many => Filter::Or(many.iter().map(|address| from(address)).collect()),
        }
    }
}

fn from(address: &str) -> Filter {
    Filter::From(TextMatch::Exact(address.to_owned()))
}

/// The VIPs the store holds, or none when it cannot say: a list that cannot be read marks
/// nobody, and is logged.
pub fn read<S: Store + ?Sized>(store: &S) -> Vips {
    Vips::new(
        store
            .vips()
            .or_log_default("the VIPs could not be read")
            .into_iter()
            .map(|vip| vip.address),
    )
}

/// Every VIP, with the name to list each by: the book's, else the one they were marked with,
/// else none.
pub fn listed(store: &dyn Store) -> Result<Vec<Vip>, CoreError> {
    Ok(store
        .vips()?
        .into_iter()
        .map(|vip| {
            let book = store
                .contact(&vip.address)
                .or_log("a VIP's contact could not be read")
                .flatten()
                .and_then(|contact| contact.name);
            Vip {
                name: book.or(vip.name),
                ..vip
            }
        })
        .collect())
}

/// Whether to mark someone a VIP or take the mark off: the two sides of one menu row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Vip,
    NotVip,
}

/// Mark `address` a VIP (`name` the name they wrote with, if any) or take the mark off.
pub fn set(
    store: &dyn Store,
    address: &str,
    name: Option<&str>,
    mark: Mark,
) -> Result<(), CoreError> {
    match mark {
        Mark::Vip => {
            store.put_vip(address, name)?;
        }
        Mark::NotVip => {
            store.delete_vip(address)?;
        }
    }
    Ok(())
}

/// What a menu row for `address` would do, given `vips`: mark them, or take the mark off.
pub fn toggled(vips: &Vips, address: &str) -> Mark {
    if vips.includes(address) {
        Mark::NotVip
    } else {
        Mark::Vip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_store::MemoryStore;

    #[test]
    fn the_list_is_folded_sorted_and_once_each() {
        let vips = Vips::new([
            " Dana@Example.test",
            "ada@example.test",
            "dana@example.test",
            " ",
        ]);
        assert_eq!(vips.addresses(), ["ada@example.test", "dana@example.test"]);
        assert!(vips.includes("DANA@example.TEST "));
        assert!(!vips.includes("dan@example.test"));
        assert!(!Vips::default().includes(""));
    }

    #[test]
    fn the_filter_is_the_whole_address_of_each() {
        let cases: [(&str, Vips, Filter); 3] = [
            ("nobody", Vips::default(), Filter::Nothing),
            ("one", Vips::new(["ada@x.test"]), from("ada@x.test")),
            (
                "two",
                Vips::new(["dana@x.test", "ada@x.test"]),
                Filter::Or(vec![from("ada@x.test"), from("dana@x.test")]),
            ),
        ];
        for (case, vips, want) in cases {
            assert_eq!(vips.filter(), want, "{case}");
        }
    }

    #[test]
    fn marking_and_unmarking_goes_through_the_store() {
        let store = MemoryStore::new();
        assert_eq!(toggled(&read(&store), "dana@x.test"), Mark::Vip);
        set(&store, "Dana@X.test", Some("Dana"), Mark::Vip).unwrap();
        assert_eq!(read(&store), Vips::new(["dana@x.test"]));
        assert_eq!(toggled(&read(&store), "dana@x.test"), Mark::NotVip);
        store
            .put_contact("dana@x.test", Some("Dana W"), &mail_store::Origin::Manual)
            .unwrap();
        assert_eq!(
            listed(&store).unwrap(),
            [Vip {
                address: "dana@x.test".to_owned(),
                name: Some("Dana W".to_owned())
            }],
            "the book's name wins"
        );
        set(&store, "dana@x.test", None, Mark::NotVip).unwrap();
        assert!(read(&store).is_empty());
    }
}
