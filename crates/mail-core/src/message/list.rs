//! A thread's way out of its mailing list, as the reader offers it.

use crate::unsubscribe::Found;
use mail_domain::Address;

/// A thread's way out of its list, with the name the front-end calls the list by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub found: Found,
    /// The list's name: its `List-Id` description, its id, else whoever sends it.
    pub list: String,
    /// Who the list's mail comes from, for "Archive all from this list".
    pub sender: Address,
    /// The address a `mailto:` unsubscribe leaves from.
    pub from: Address,
}

/// The name a list goes by: its `List-Id` description, its id, else whoever sends it.
fn list_name(found: &Found, sender: &Address) -> String {
    match &found.list.id {
        Some(id) => id.description.clone().unwrap_or_else(|| id.id.clone()),
        None => sender
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| sender.email.clone()),
    }
}

/// The offer for `found`, or `None` when it has no way out this client can use.
pub fn offer_of(found: Found, sender: Address, from: Address) -> Option<Offer> {
    found.list.preferred()?;
    Some(Offer {
        list: list_name(&found, &sender),
        found,
        sender,
        from,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;
    use mail_mime::{ListHeaders, ListId};

    const ONE_CLICK: &str = "List-Id: Rust Weekly <weekly.rust.test>\r\n\
         List-Unsubscribe: <https://lists.rust.test/u/abc>, <mailto:leave@rust.test>\r\n\
         List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n";
    const MAILTO: &str = "List-Id: <announce.example.test>\r\n\
         List-Unsubscribe: <mailto:leave@example.test?subject=remove>\r\n";
    const WEB: &str = "List-Id: Deals <deals.shop.test>\r\n\
         List-Unsubscribe: <https://www.shop.test/prefs?u=1>\r\n";
    const NO_WAY_OUT: &str = "List-Id: Quiet <quiet.example.test>\r\n";

    fn address(email: &str) -> Address {
        Address {
            name: None,
            email: email.to_owned(),
        }
    }

    fn account() -> porter_core::AccountId {
        account_id_from_uuid(uuid::Uuid::from_u128(0xa1))
    }

    fn offer_from(raw_headers: &str) -> Option<Offer> {
        let bytes = format!("From: news@example.test\r\n{raw_headers}\r\nbody\r\n");
        let found = Found {
            message: mail_domain::MessageId::generate(),
            account: account(),
            addressed: vec![address("me@example.test")],
            list: mail_mime::list_headers(bytes.as_bytes()),
        };
        offer_of(
            found,
            address("news@example.test"),
            address("me@example.test"),
        )
    }

    #[test]
    fn there_is_an_offer_only_when_there_is_a_way_out() {
        let cases = [
            (ONE_CLICK, true),
            (MAILTO, true),
            (WEB, true),
            (NO_WAY_OUT, false),
            ("", false),
        ];
        for (headers, offered) in cases {
            assert_eq!(offer_from(headers).is_some(), offered, "{headers:?}");
        }
    }

    #[test]
    fn the_list_is_named_by_its_description_then_its_id_then_its_sender() {
        let named = |id: Option<ListId>| {
            let found = Found {
                message: mail_domain::MessageId::generate(),
                account: account(),
                addressed: vec![],
                list: ListHeaders {
                    id,
                    unsubscribe: vec![mail_mime::Unsubscribe::Web {
                        url: "https://example.test/".to_owned(),
                    }],
                },
            };
            let sender = Address {
                name: Some("News".to_owned()),
                email: "news@example.test".to_owned(),
            };
            offer_of(found, sender, address("me@example.test"))
                .unwrap()
                .list
        };
        let id = |description: Option<&str>| ListId {
            description: description.map(str::to_owned),
            id: "weekly.example.test".to_owned(),
        };
        assert_eq!(named(Some(id(Some("Weekly")))), "Weekly");
        assert_eq!(named(Some(id(None))), "weekly.example.test");
        assert_eq!(named(None), "News");
    }
}
