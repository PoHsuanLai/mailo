//! People's cards: stored, listed, found by address, replaced and forgotten the same way by the
//! SQLite store and the in-memory one.

use crate::support::*;
use mail_store::{CardId, Edit, GroupHome, PersonCard};

fn card(id: CardId, name: &str, addresses: &[&str], home: GroupHome) -> PersonCard {
    PersonCard {
        id,
        name: name.to_owned(),
        addresses: addresses.iter().map(|a| (*a).to_owned()).collect(),
        vcard: format!("BEGIN:VCARD\r\nVERSION:4.0\r\nFN:{name}\r\nEND:VCARD\r\n"),
        home,
    }
}

#[test]
fn cards_are_kept_listed_found_by_address_replaced_and_forgotten_alike() {
    let (a, b) = both(|store| {
        let before = store.cards().unwrap();
        let ada = card(
            CardId::local("ada-1"),
            "ada Lovelace",
            &["ada@example.test", "countess@example.test"],
            GroupHome::Local,
        );
        let synced = card(
            CardId("https://dav.example.test/book/bob.vcf".to_owned()),
            "Bob",
            &["bob@example.test", "ada@example.test"],
            GroupHome::Book {
                url: "https://dav.example.test/book/".to_owned(),
                href: "https://dav.example.test/book/bob.vcf".to_owned(),
                edit: Edit::Synced,
            },
        );
        let nobody = card(CardId::local("zed"), "Žofia", &[], GroupHome::Local);
        for each in [&ada, &synced, &nobody] {
            store.put_card(each).unwrap();
        }
        let listed: Vec<String> = store.cards().unwrap().into_iter().map(|c| c.name).collect();
        let found = |address: &str| store.card_of(address).unwrap().map(|c| c.id);
        let by_address = (
            found("COUNTESS@example.test"),
            found("ada@example.test"),
            found("carol@example.test"),
            found("not an address"),
        );

        let mut moved = ada.clone();
        moved.addresses = vec!["ada@elsewhere.test".to_owned()];
        store.put_card(&moved).unwrap();
        let after_move = (found("countess@example.test"), found("ada@elsewhere.test"));

        let forgot = store.delete_card(&synced.id).unwrap();
        let again = store.delete_card(&synced.id).unwrap();
        let gone = (store.card(&synced.id).unwrap(), found("bob@example.test"));
        let kept = store.card(&ada.id).unwrap();
        (
            before, listed, by_address, after_move, forgot, again, gone, kept,
        )
    });
    assert_eq!(a, b);
    let (before, listed, by_address, after_move, forgot, again, gone, kept) = a;
    assert!(before.is_empty());
    // ASCII case folded, anything else by its bytes: "Ž" sorts after every ASCII letter.
    assert_eq!(listed, ["ada Lovelace", "Bob", "Žofia"]);
    let bob = CardId("https://dav.example.test/book/bob.vcf".to_owned());
    assert_eq!(
        by_address,
        (
            Some(CardId::local("ada-1")),
            // Two cards list it: the first by id.
            Some(bob.clone()).min(Some(CardId::local("ada-1"))),
            None,
            None
        )
    );
    assert_eq!(after_move, (None, Some(CardId::local("ada-1"))));
    assert!(forgot);
    assert!(!again);
    assert_eq!(gone, (None, None));
    assert_eq!(kept.unwrap().addresses, ["ada@elsewhere.test"]);
}
