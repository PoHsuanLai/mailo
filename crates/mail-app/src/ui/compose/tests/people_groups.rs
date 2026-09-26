//! A contact group in the To field's menu: offered above the people for a word of its name, and
//! choosing it puts each member on the message once.

use super::super::page::{Float, List};
use super::super::recipients::{people_items, pick_person, typed};
use super::*;
use crate::ui::contacts::tests::{ADDED, HEARD, WRITTEN, the_book};
use mail_store::{Group, GroupHome, GroupId, Store};

#[test]
fn a_group_is_offered_first_and_choosing_it_puts_each_member_on_once() {
    let (store, _dir) = the_book();
    let mut page = Page::of(&draft_of(""), Vec::new(), Vec::new());
    // Daniel is on the message already: the group adds only who is not.
    typed(&mut page, List::To, "dan".to_owned(), store.as_ref());
    pick_person(&mut page, List::To, WRITTEN);
    let before = page.to.len();
    let uid = "urn:uuid:5c1f6e1a-0000-4000-8000-00000000d0d0";
    store
        .put_group(&Group {
            id: GroupId::local(uid),
            uid: Some(uid.to_owned()),
            name: "Design review".to_owned(),
            members: vec![
                format!("mailto:{WRITTEN}"),
                format!("mailto:{ADDED}"),
                format!("mailto:{HEARD}"),
                "urn:uuid:ffffffff-0000-4000-8000-000000000000".to_owned(),
            ],
            home: GroupHome::Local,
        })
        .unwrap();

    typed(&mut page, List::To, "design".to_owned(), store.as_ref());
    let items = people_items(&page, List::To);
    assert_eq!(items[0].name, "Design review");
    assert_eq!(
        items[0].help.as_deref(),
        Some("Group · 3 people · 1 not found")
    );
    assert_eq!(
        page.float,
        Float::People {
            list: List::To,
            active: 0
        }
    );
    let key = items[0].key.clone();
    pick_person(&mut page, List::To, &key);
    let on: Vec<&str> = page.to.iter().map(|p| p.address.as_str()).collect();
    assert_eq!(on, [WRITTEN, ADDED, HEARD]);
    assert_eq!(page.to.len(), before + 2);
    assert_eq!(page.typed_to, "");
    assert_eq!(page.float, Float::Closed);
}
