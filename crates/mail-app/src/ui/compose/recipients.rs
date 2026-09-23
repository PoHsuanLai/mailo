//! The To and Cc rows: chips, what is typed beside them, and the people menu under them.

use super::super::menu::MenuItem;
use super::float::people_rows;
use super::page::{CcRow, Float, List, Page, person};
use crate::editor::{Person, resolve};

/// Someone typed into `list`'s field. A comma commits what came before it.
pub(in crate::ui) fn typed(page: &mut Page, list: List, value: String) {
    if let Some(done) = value.strip_suffix(',') {
        *page.typed_mut(list) = done.to_owned();
        commit_typed(page, list);
        return;
    }
    *page.typed_mut(list) = value;
    page.float = if people_items(page, list).is_empty() {
        Float::Closed
    } else {
        Float::People { list, active: 0 }
    };
}

/// People matching what is typed in `list`, less those already on the message.
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
    people_rows(
        resolve(typed, &page.people)
            .into_iter()
            .filter(|person| !listed(person))
            .collect(),
    )
}

/// Add the suggested person at `address` to `list`.
pub(in crate::ui) fn pick_person(page: &mut Page, list: List, address: &str) {
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
