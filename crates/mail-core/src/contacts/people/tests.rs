//! People's cards against a real store: made, edited onto what the card already says, forgotten,
//! added from mail, and the book autocomplete ranks kept in step.

use super::*;
use mail_store::SqliteStore;

const BOOK: &str = "https://dav.example.test/book/";
const PHOTO: &str = "data:image/png;base64,iVBORw0KGgo=";

fn store() -> SqliteStore {
    mail_store::testing::in_memory()
}

/// A synced card, as a sync keeps it, with a type on its address and a property nothing reads.
fn synced(store: &dyn Store) -> PersonCard {
    let text = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:grace-1\r\nFN:Grace Hopper\r\n\
                N:Hopper;Grace;;;\r\nEMAIL;TYPE=work;PREF=1:grace@example.test\r\n\
                TEL;TYPE=cell:+1 555\r\nBDAY:19061209\r\nEND:VCARD\r\n";
    let card = vcard::parse(text).remove(0);
    let href = format!("{BOOK}grace.vcf");
    let person = person_card(
        &card,
        text.to_owned(),
        CardId(href.clone()),
        GroupHome::Book {
            url: BOOK.to_owned(),
            href,
            edit: Edit::Synced,
        },
    );
    store.put_card(&person).unwrap();
    store
        .put_contact(
            "grace@example.test",
            Some("Grace Hopper"),
            &Origin::Book(format!("carddav:{BOOK}")),
        )
        .unwrap();
    person
}

fn origin(store: &dyn Store, address: &str) -> Option<(Option<String>, Origin)> {
    store
        .contact(address)
        .unwrap()
        .map(|contact| (contact.name, contact.origin))
}

#[test]
fn a_new_card_is_listed_says_what_it_was_given_and_names_its_addresses() {
    let store = store();
    let given = Details {
        name: "Ada Lovelace".to_owned(),
        emails: vec!["Ada@Example.test".to_owned(), " ".to_owned()],
        phones: vec!["+44 20 1".to_owned()],
        postal: vec![Postal {
            street: "12 St James's Square".to_owned(),
            locality: "London".to_owned(),
            ..Postal::default()
        }],
        org: "Analytical Engines".to_owned(),
        note: "Wrote the first program.".to_owned(),
        photo: Some(PHOTO.to_owned()),
    };
    let made = create(&store, &given).unwrap();
    assert_eq!(made.home, GroupHome::Local);
    assert_eq!(made.addresses, ["ada@example.test"]);
    assert_eq!(listed(&store, "lov").unwrap(), std::slice::from_ref(&made));
    assert_eq!(
        listed(&store, "example").unwrap(),
        std::slice::from_ref(&made)
    );
    assert!(listed(&store, "grace").unwrap().is_empty());
    let back = details(&made);
    assert_eq!(
        back,
        Details {
            emails: vec!["Ada@Example.test".to_owned()],
            ..given
        },
        "the blank address is left out"
    );
    assert_eq!(
        origin(&store, "ada@example.test"),
        Some((Some("Ada Lovelace".to_owned()), Origin::Manual))
    );
    assert_eq!(photo_of(&store, "ADA@example.test").as_deref(), Some(PHOTO));
}

#[test]
fn an_edit_is_made_onto_the_card_and_a_synced_one_is_marked_for_writing_back() {
    let store = store();
    let held = synced(&store);
    let mut edit = details(&held);
    edit.emails.push("amazing.grace@example.test".to_owned());
    edit.note = "Rear admiral.".to_owned();
    let saved = save(&store, &held.id, &edit).unwrap();
    assert!(matches!(
        saved.home,
        GroupHome::Book {
            edit: Edit::Edited,
            ..
        }
    ));
    let card = vcard::parse(&saved.vcard).remove(0);
    assert_eq!(
        card.emails[0].kinds,
        ["work"],
        "a kept address keeps its type"
    );
    assert_eq!(card.emails[0].pref, Some(1));
    assert_eq!(card.phones[0].kinds, ["cell"]);
    assert!(card.name.is_some(), "the name did not change, so N stays");
    assert!(
        card.other.iter().any(|line| line.name == "BDAY"),
        "what the editor does not show rides along"
    );
    assert_eq!(card.note.as_deref(), Some("Rear admiral."));
    assert_eq!(
        origin(&store, "amazing.grace@example.test"),
        Some((
            Some("Grace Hopper".to_owned()),
            Origin::Book(format!("carddav:{BOOK}"))
        )),
        "a synced card's new address is its book's"
    );

    edit.name = "Admiral Hopper".to_owned();
    edit.emails = vec!["amazing.grace@example.test".to_owned()];
    let saved = save(&store, &held.id, &edit).unwrap();
    let card = vcard::parse(&saved.vcard).remove(0);
    assert_eq!(card.name, None, "N no longer says the name");
    assert_eq!(saved.name, "Admiral Hopper");
    assert_eq!(
        origin(&store, "grace@example.test"),
        None,
        "an address taken off, that no mail knew, goes"
    );
    assert!(matches!(
        forget(&store, &held.id),
        Err(PersonError::InBook { name }) if name == "Admiral Hopper"
    ));
}

#[test]
fn edits_that_cannot_be_made_say_why() {
    let store = store();
    let cases: Vec<(&str, Details, &str)> = vec![
        ("nothing to call it", Details::default(), "empty"),
        (
            "an address that is not one",
            Details {
                name: "Ada".to_owned(),
                emails: vec!["not an address".to_owned()],
                ..Details::default()
            },
            "bad",
        ),
    ];
    for (name, given, want) in cases {
        let got = match create(&store, &given) {
            Err(PersonError::Empty) => "empty",
            Err(PersonError::BadAddress(_)) => "bad",
            other => panic!("{name}: {other:?}"),
        };
        assert_eq!(got, want, "{name}");
    }
    assert!(matches!(
        save(
            &store,
            &CardId::local("nobody"),
            &Details::of("A", "a@example.test")
        ),
        Err(PersonError::Gone)
    ));
    assert!(store.cards().unwrap().is_empty(), "nothing was kept");
}

#[test]
fn a_card_forgotten_hands_its_addresses_back_to_the_mail_that_knew_them() {
    let store = store();
    let made = create(&store, &Details::of("Bob", "bob@example.test")).unwrap();
    store
        .put_contact("bob@example.test", None, &Origin::Manual)
        .unwrap();
    forget(&store, &made.id).unwrap();
    assert_eq!(store.card(&made.id).unwrap(), None);
    assert_eq!(origin(&store, "bob@example.test"), None);
    assert!(matches!(forget(&store, &made.id), Err(PersonError::Gone)));
}

#[test]
fn a_sender_is_added_once_named_as_the_book_names_them() {
    let store = store();
    store
        .put_contact("cy@example.test", Some("Cy Twombly"), &Origin::History)
        .unwrap();
    let Added::New(made) = add_sender(&store, "Cy@example.test", "cy").unwrap() else {
        panic!("a new card");
    };
    assert_eq!(made.name, "Cy Twombly");
    assert!(matches!(
        add_sender(&store, "cy@example.test", "Someone").unwrap(),
        Added::Already(held) if held.id == made.id
    ));
    assert_eq!(store.cards().unwrap().len(), 1);
}

#[test]
fn only_a_photo_written_into_the_card_is_shown() {
    let store = store();
    let cases = [
        (Some(PHOTO), Some(PHOTO)),
        (Some("https://tracker.example/p.png"), None),
        (None, None),
    ];
    for (n, (photo, want)) in cases.into_iter().enumerate() {
        let address = format!("p{n}@example.test");
        create(
            &store,
            &Details {
                photo: photo.map(str::to_owned),
                ..Details::of("P", &address)
            },
        )
        .unwrap();
        assert_eq!(photo_of(&store, &address).as_deref(), want, "{photo:?}");
    }
    assert_eq!(photo_of(&store, "nobody@example.test"), None);
}
