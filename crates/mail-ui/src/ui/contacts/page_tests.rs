//! Settings › Contacts over the reference fixture: a card made from New Contact, edited from its
//! row and deleted after asking; a group made from New Group and given a member.

use super::key_of;
use crate::ui::copy;
use crate::ui::fixtures::{click, drain_seen, type_into, work};
use crate::ui::settings_window::tests::opened_on;
use crate::ui::view::PageId;
use mail_core::Store;

#[test]
fn a_contact_is_made_edited_and_deleted_from_its_page() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::CONTACTS);
    let seen = click(&mut dom, seen.one("id", super::NEW_ROW)).merge(drain_seen(&mut dom));
    let seen = type_into(&mut dom, seen.one("id", "contact-name"), "Grace Hopper").merge(seen);
    let seen = type_into(
        &mut dom,
        seen.one("id", "contact-email-0"),
        "grace@example.test",
    )
    .merge(seen);
    assert_eq!(
        built.store.card_of("grace@example.test").unwrap(),
        None,
        "kept before Add Contact"
    );
    let seen = click(&mut dom, seen.one("id", "contact-save"))
        .merge(drain_seen(&mut dom))
        .merge(seen);
    let card = built
        .store
        .card_of("grace@example.test")
        .unwrap()
        .expect("Add Contact kept the card");
    assert_eq!(card.name, "Grace Hopper");

    let row = format!("contacts-pane-card-{}", key_of(&card.id.0));
    let seen = click(&mut dom, seen.one("id", &row))
        .merge(drain_seen(&mut dom))
        .merge(seen);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("value=\"Grace Hopper\""), "its page: {page}");
    let phone = *seen
        .all("id", "contact-phone-0")
        .last()
        .expect("the phone field");
    let seen = type_into(&mut dom, phone, "+1 555 0100").merge(seen);
    let save = *seen.all("id", "contact-save").last().expect("Save");
    let seen = click(&mut dom, save)
        .merge(drain_seen(&mut dom))
        .merge(seen);
    let kept = mail_core::contacts::people::details(&built.store.card(&card.id).unwrap().unwrap());
    assert_eq!(kept.phones, ["+1 555 0100"]);
    assert_eq!(
        kept.emails,
        ["grace@example.test"],
        "what was not touched stays"
    );

    let delete = seen.one("aria-label", &copy::confirm::delete_contact("Grace Hopper"));
    let seen = click(&mut dom, delete).merge(drain_seen(&mut dom));
    assert!(
        built.store.card(&card.id).unwrap().is_some(),
        "deleted before asking"
    );
    let confirm = *seen
        .all("data-role", "destructive")
        .last()
        .expect("the alert's Delete");
    click(&mut dom, confirm);
    assert_eq!(built.store.card(&card.id).unwrap(), None, "the card stayed");
}

#[test]
fn a_group_is_made_and_given_members_from_its_page() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::CONTACTS);
    let before = built.store.groups().unwrap().len();
    let seen =
        click(&mut dom, seen.one("id", "contacts-pane-new-group")).merge(drain_seen(&mut dom));
    let groups = built.store.groups().unwrap();
    assert_eq!(groups.len(), before + 1, "New Group made no group");
    let made = groups
        .iter()
        .find(|group| group.members.is_empty())
        .expect("the new group")
        .clone();
    let seen = type_into(&mut dom, seen.one("id", "group-name"), "Team").merge(seen);
    let seen = type_into(
        &mut dom,
        seen.one("id", "group-add"),
        "ada@example.test, bob@example.test",
    )
    .merge(seen);
    click(&mut dom, seen.one("id", "group-add-button"));
    let group = built.store.group(&made.id).unwrap().unwrap();
    assert_eq!(
        group.members,
        ["mailto:ada@example.test", "mailto:bob@example.test"]
    );
}
