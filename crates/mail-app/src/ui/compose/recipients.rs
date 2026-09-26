//! The To and Cc rows: chips, what is typed beside them, and the people menu under them.
//!
//! The menu offers the contact groups whose name fits what was typed above the people, and
//! choosing a group puts each of its members on the message (`contacts::groups`).

use mail_store::Store;

use super::super::contacts::book::suggest;
use super::super::contacts::groups::{self, Offer};
use super::super::menu::{MenuItem, Right, Tile};
use super::float::people_rows;
use super::page::{CcRow, Float, List, Page, person};
use crate::editor::Person;

/// Someone typed into `list`'s field. A comma commits what came before it.
///
/// The suggestions are asked of the contact book here, on the keystroke, not debounced. One
/// read, measured in a debug build (`contacts::tests::a_suggestion_is_cheap_enough_for_every_
/// keystroke`): under 0.7 ms at a thousand contacts for any text, and at ten thousand under
/// 1 ms for most and about 6 ms for a first letter or a miss. The store's scale fixture's ten
/// thousand messages come from 97 senders. A menu that lags the field by a quiet period would
/// offer people for text that is no longer there.
pub(in crate::ui) fn typed(page: &mut Page, list: List, value: String, store: &dyn Store) {
    if let Some(done) = value.strip_suffix(',') {
        *page.typed_mut(list) = done.to_owned();
        commit_typed(page, list);
        return;
    }
    (page.people, page.groups) = if value.trim().is_empty() {
        (Vec::new(), Vec::new())
    } else {
        (suggest(store, &value), groups::offers(store, &value))
    };
    *page.typed_mut(list) = value;
    page.float = if people_items(page, list).is_empty() {
        Float::Closed
    } else {
        Float::People { list, active: 0 }
    };
}

/// What a group's row in the menu is keyed by: nothing an address can be, since an address's
/// local part holds no unquoted `:`.
const GROUP_KEY: &str = "group:";

/// The groups offered for what is typed, then the book's suggestions in its order less those
/// already on the message.
pub(in crate::ui) fn people_items(page: &Page, list: List) -> Vec<MenuItem> {
    let typed = match list {
        List::To => &page.typed_to,
        List::Cc => &page.typed_cc,
    };
    if typed.trim().is_empty() {
        return Vec::new();
    }
    let listed = |person: &Person| {
        page.to
            .iter()
            .chain(&page.cc)
            .any(|other| other.address.eq_ignore_ascii_case(&person.address))
    };
    let mut items: Vec<MenuItem> = page.groups.iter().map(group_row).collect();
    items.extend(people_rows(
        page.people
            .iter()
            .filter(|person| !listed(person))
            .collect(),
    ));
    items
}

/// A group's row: its name, and how many people it adds.
fn group_row(offer: &Offer) -> MenuItem {
    let count = offer.expanded.people.len();
    let mut help = if count == 1 {
        "Group · 1 person".to_owned()
    } else {
        format!("Group · {count} people")
    };
    let missing = offer.expanded.unresolved.len();
    if missing > 0 {
        help.push_str(&format!(" · {missing} not found"));
    }
    MenuItem {
        key: format!("{GROUP_KEY}{}", offer.id),
        tile: Tile::Icon(ds::Icon::Group),
        name: offer.name.clone(),
        help: Some(help),
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// Add the suggested person at `address` to `list`, or every member of the group it keys.
pub(in crate::ui) fn pick_person(page: &mut Page, list: List, address: &str) {
    if let Some(id) = address.strip_prefix(GROUP_KEY) {
        let Some(offer) = page.groups.iter().find(|g| g.id.0 == id).cloned() else {
            return;
        };
        for person in offer.expanded.people {
            add(page, list, person);
        }
        page.typed_mut(list).clear();
        page.float = Float::Closed;
        return;
    }
    let Some(found) = page
        .people
        .iter()
        .find(|person| person.address == address)
        .cloned()
    else {
        return;
    };
    add(page, list, found);
    page.typed_mut(list).clear();
    page.float = Float::Closed;
}

/// Turn what is typed in `list` into chips. An entry that is not an address stays typed and
/// says why, rather than vanishing or being guessed at.
pub(in crate::ui) fn commit_typed(page: &mut Page, list: List) -> bool {
    let typed = page.typed_mut(list).clone();
    if typed.trim().is_empty() {
        return true;
    }
    match crate::view::parse_addresses(&typed) {
        Ok(addresses) => {
            for address in &addresses {
                add(page, list, person(address));
            }
            page.typed_mut(list).clear();
            page.float = Float::Closed;
            page.notice = None;
            true
        }
        Err(why) => {
            page.notice = Some(why);
            false
        }
    }
}

fn add(page: &mut Page, list: List, joining: Person) {
    let listed = page
        .to
        .iter()
        .chain(&page.cc)
        .any(|other| other.address.eq_ignore_ascii_case(&joining.address));
    if listed {
        return;
    }
    if list == List::Cc {
        page.cc_row = CcRow::Shown;
    }
    page.flash = Some(joining.address.clone());
    page.list_mut(list).push(joining);
    page.touch();
}

/// Take `address` off `list`.
pub(in crate::ui) fn remove(page: &mut Page, list: List, address: &str) {
    page.list_mut(list)
        .retain(|person| person.address != address);
    page.touch();
}

/// Backspace in an empty field takes the last chip.
pub(in crate::ui) fn pop_last(page: &mut Page, list: List) {
    if page.typed_mut(list).is_empty() && page.list_mut(list).pop().is_some() {
        page.touch();
    }
}
