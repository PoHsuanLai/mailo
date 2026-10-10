//! Contact groups against a real store: who a group expands to, which groups To offers, and the
//! edits — a synced group's marked for writing back.

use super::*;
use mail_store::{AddressBook, BookCard, Origin, SqliteStore};

const BOOK: &str = "https://dav.example.test/book/";
const GRACE_UID: &str = "urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1";
const NOBODY: &str = "urn:uuid:ffffffff-0000-4000-8000-000000000000";
/// Added by hand and never written to.
const ADDED: &str = "dara.quinn@example.test";
const WRITTEN: &str = "daniel@example.test";

/// A store whose book holds two people by hand.
fn the_book() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .put_contact(ADDED, Some("Dara Quinn"), &Origin::Manual)
        .unwrap();
    store
        .put_contact(WRITTEN, Some("Daniel Brook"), &Origin::Manual)
        .unwrap();
    (store, dir)
}

/// A synced book holding Grace's card, whose `UID` a member names. Stored the way a sync before
/// `BookCard::uid` existed stored it, so only the card's text says the `UID`.
fn with_grace(store: &dyn Store) {
    let vcard = format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{}\r\nFN:Grace Hopper\r\n\
         EMAIL:grace@example.test\r\nEND:VCARD\r\n",
        GRACE_UID.to_uppercase().replace("URN:UUID:", "")
    );
    let book = AddressBook {
        url: BOOK.to_owned(),
        cards: [(
            format!("{BOOK}grace.vcf"),
            BookCard {
                uid: None,
                etag: "\"1\"".to_owned(),
                addresses: vec!["grace@example.test".to_owned()],
                vcard,
            },
        )]
        .into(),
        ..AddressBook::default()
    };
    store.put_address_book(&book).unwrap();
    store
        .put_contact(
            "grace@example.test",
            Some("Grace Hopper"),
            &Origin::Book(format!("carddav:{BOOK}")),
        )
        .unwrap();
}

fn group(uid: &str, name: &str, members: &[&str], home: GroupHome) -> Group {
    Group {
        id: match &home {
            GroupHome::Local => GroupId::local(uid),
            GroupHome::Book { href, .. } => GroupId(href.clone()),
        },
        uid: Some(uid.to_owned()),
        name: name.to_owned(),
        members: members.iter().map(|m| (*m).to_owned()).collect(),
        home,
    }
}

fn addresses(people: &[Person]) -> Vec<&str> {
    people.iter().map(|p| p.address.as_str()).collect()
}

#[test]
fn a_group_expands_by_address_by_card_and_by_nested_group_and_keeps_what_it_cannot_find() {
    let (store, _dir) = the_book();
    with_grace(&store);
    let inner = group(
        "inner-1",
        "Inner",
        &[&format!("mailto:{WRITTEN}"), "mailto:Grace@Example.test"],
        GroupHome::Local,
    );
    let outer = group(
        "outer-1",
        "Outer",
        &[
            &format!("mailto:{ADDED}"),
            GRACE_UID,
            "urn:uuid:inner-1",
            NOBODY,
            "mailto:",
            // Names itself: nothing more to add, and no endless walk.
            "outer-1",
        ],
        GroupHome::Local,
    );
    store.put_group(&inner).unwrap();
    store.put_group(&outer).unwrap();

    let expanded = expand(&store, &outer);
    assert_eq!(
        addresses(&expanded.people),
        [ADDED, "grace@example.test", WRITTEN],
        "each address once, in the group's order"
    );
    assert_eq!(expanded.people[1].name, "Grace Hopper");
    assert_eq!(expanded.unresolved, [NOBODY, "mailto:"]);
    // Expanding changes nothing stored: the unresolved member is still a member.
    assert_eq!(store.group(&outer.id).unwrap(), Some(outer));
}

#[test]
fn to_offers_a_group_by_a_word_of_its_name_and_not_one_of_nobody() {
    let (store, _dir) = the_book();
    let before = offers(&store, "fam");
    let family = group(
        "fam-1",
        "The Family",
        &[&format!("mailto:{WRITTEN}"), &format!("mailto:{ADDED}")],
        GroupHome::Local,
    );
    store.put_group(&family).unwrap();
    store
        .put_group(&group("fam-2", "Family nobody", &[], GroupHome::Local))
        .unwrap();
    store
        .put_group(&group(
            "fam-3",
            "Family unknown",
            &[NOBODY],
            GroupHome::Local,
        ))
        .unwrap();
    let after = offers(&store, "fam");
    assert!(before.is_empty());
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after[0].id, family.id);
    assert_eq!(addresses(&after[0].expanded.people), [WRITTEN, ADDED]);
    assert!(
        offers(&store, "amily").is_empty(),
        "a word's start, not its middle"
    );
    assert_eq!(offers(&store, "the fam").len(), 1);
    assert_eq!(
        listed(&store, "fam").unwrap().len(),
        3,
        "the sheet lists them all"
    );
}

#[test]
fn an_edit_to_a_synced_group_is_marked_for_the_next_sync_and_a_local_one_is_just_kept() {
    let (store, _dir) = the_book();
    let href = format!("{BOOK}team.vcf");
    let synced = group(
        "team-1",
        "Team",
        &[NOBODY],
        GroupHome::Book {
            url: BOOK.to_owned(),
            href: href.clone(),
            edit: Edit::Synced,
        },
    );
    store.put_group(&synced).unwrap();
    let local = create(&store, "  Lunch  ").unwrap();
    assert_eq!(local.name, "Lunch");
    assert!(local.members.is_empty(), "a new group has nobody yet");
    assert!(matches!(create(&store, "  "), Err(GroupError::NoName)));

    let added = add(&store, &synced.id, &format!("{ADDED}, Daniel <{WRITTEN}>")).unwrap();
    assert_eq!(
        added.members,
        [
            NOBODY.to_owned(),
            format!("mailto:{ADDED}"),
            format!("mailto:{WRITTEN}")
        ],
        "the unresolved member stays"
    );
    assert_eq!(
        added.home,
        GroupHome::Book {
            url: BOOK.to_owned(),
            href,
            edit: Edit::Edited
        }
    );
    let twice = add(&store, &synced.id, ADDED).unwrap();
    assert_eq!(
        twice.members.len(),
        3,
        "an address already in it is not added again"
    );
    let removed = remove(&store, &synced.id, &format!("mailto:{ADDED}")).unwrap();
    assert_eq!(removed.members.len(), 2);
    let renamed = rename(&store, &local.id, "Lunch club").unwrap();
    assert_eq!(renamed.home, GroupHome::Local);
    assert!(matches!(
        rename(&store, &local.id, "  "),
        Err(GroupError::NoName)
    ));
    assert!(matches!(
        rename(&store, &GroupId::local("urn:uuid:none"), "Nobody"),
        Err(GroupError::Gone)
    ));

    assert!(
        matches!(forget(&store, &synced.id), Err(GroupError::InBook { name }) if name == "Team"),
        "a synced group is its book's to delete"
    );
    let before = store.groups().unwrap().len();
    forget(&store, &local.id).unwrap();
    assert_eq!(store.groups().unwrap().len(), before - 1);
}

#[test]
fn a_member_is_labelled_by_whom_it_names() {
    let (store, _dir) = the_book();
    with_grace(&store);
    let g = group(
        "g",
        "G",
        &[GRACE_UID, NOBODY, &format!("mailto:{ADDED}")],
        GroupHome::Local,
    );
    let labels: Vec<Labelled> = members(&store, &g)
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(
        labels[0],
        Labelled::Named(Person {
            name: "Grace Hopper".to_owned(),
            address: "grace@example.test".to_owned()
        })
    );
    assert_eq!(labels[1], Labelled::NotFound);
    assert!(
        matches!(&labels[2], Labelled::Named(person) if person.address == ADDED),
        "{labels:?}"
    );
}
