//! Contact groups in the window, against a real store: who a group expands to, which groups To
//! offers, and the sheet's edits — a synced group's marked for writing back.

use mail_store::{AddressBook, BookCard, Edit, Group, GroupHome, GroupId, Origin, Store};

use super::groups::{self, expand, offers};
use super::tests::{ADDED, WRITTEN, the_book};
use crate::editor::Person;

const BOOK: &str = "https://dav.example.test/book/";
const GRACE_UID: &str = "urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1";
const NOBODY: &str = "urn:uuid:ffffffff-0000-4000-8000-000000000000";

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
    with_grace(store.as_ref());
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

    let expanded = expand(store.as_ref(), &outer);
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
    let before = offers(store.as_ref(), "fam");
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
    let after = offers(store.as_ref(), "fam");
    assert!(before.is_empty());
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after[0].id, family.id);
    assert_eq!(addresses(&after[0].expanded.people), [WRITTEN, ADDED]);
    assert!(
        offers(store.as_ref(), "amily").is_empty(),
        "a word's start, not its middle"
    );
    assert_eq!(offers(store.as_ref(), "the fam").len(), 1);
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
    let local = groups::create(store.as_ref(), "  Lunch  ").unwrap();
    assert_eq!(local.name, "Lunch");
    assert!(local.members.is_empty(), "a new group has nobody yet");

    let added = groups::add(
        store.as_ref(),
        &synced.id,
        &format!("{ADDED}, Daniel <{WRITTEN}>"),
    )
    .unwrap();
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
    let twice = groups::add(store.as_ref(), &synced.id, ADDED).unwrap();
    assert_eq!(
        twice.members.len(),
        3,
        "an address already in it is not added again"
    );
    let removed = groups::remove(store.as_ref(), &synced.id, &format!("mailto:{ADDED}")).unwrap();
    assert_eq!(removed.members.len(), 2);
    let renamed = groups::rename(store.as_ref(), &local.id, "Lunch club").unwrap();
    assert_eq!(renamed.home, GroupHome::Local);
    assert!(groups::rename(store.as_ref(), &local.id, "  ").is_err());

    assert!(
        groups::forget(store.as_ref(), &synced.id).is_err(),
        "a synced group is its book's to delete"
    );
    let before = store.groups().unwrap().len();
    groups::forget(store.as_ref(), &local.id).unwrap();
    assert_eq!(store.groups().unwrap().len(), before - 1);
}

#[test]
fn a_member_is_labelled_by_whom_it_names() {
    let (store, _dir) = the_book();
    with_grace(store.as_ref());
    let g = group(
        "g",
        "G",
        &[GRACE_UID, NOBODY, &format!("mailto:{ADDED}")],
        GroupHome::Local,
    );
    let labels: Vec<String> = groups::member_labels(store.as_ref(), &g)
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(labels[0], "Grace Hopper <grace@example.test>");
    assert_eq!(labels[1], format!("{NOBODY} (not found)"));
    assert!(labels[2].ends_with(&format!("<{ADDED}>")), "{labels:?}");
}
