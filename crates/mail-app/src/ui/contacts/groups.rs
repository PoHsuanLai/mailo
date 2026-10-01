//! Contact groups in the window, as functions of a store: who a group is, which groups to offer
//! for what was typed in To, and the edits a person makes in the Contacts sheet.
//!
//! Who a group is gets decided when it is used, from its `MEMBER` URIs (RFC 6350 §6.6.5): a
//! `mailto:` is its address; a `urn:uuid:` (or any other URI) is the card of a synced address
//! book with that `UID`, or another group with it, whose members join in turn. A member naming
//! nothing known is counted and left alone — it stays in the group and goes back to the server
//! with it — and a group of nobody is still a group.

use std::collections::BTreeSet;

use mail_domain::filter::search_tokens;
use mail_pim::vcard::{self, Member};
use mail_store::{Edit, Group, GroupHome, GroupId, Store};

use super::book::person_of;
use crate::editor::Person;

/// How many groups the To and Cc menus offer above the people.
pub(in crate::ui) const OFFERED: usize = 3;

/// Who a group stands for now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) struct Expanded {
    /// Everyone it names, in its order, each address once.
    pub people: Vec<Person>,
    /// Its members that name nobody known here, as written.
    pub unresolved: Vec<String>,
}

/// A group the composer offers, expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Offer {
    pub id: GroupId,
    pub name: String,
    pub expanded: Expanded,
}

/// A card of a synced book that a member may name: its `UID` as [`vcard::uid_key`] folds it,
/// its name, and its preferred address.
struct Named {
    key: String,
    name: Option<String>,
    address: String,
}

/// What members are resolved against: every synced card with a `UID` and an address, and every
/// group.
struct Index {
    cards: Vec<Named>,
    groups: Vec<Group>,
}

impl Index {
    fn of(store: &dyn Store) -> Self {
        let mut cards = Vec::new();
        for book in store.address_books().unwrap_or_default() {
            for held in book.cards.values() {
                // A card stored before its `UID` was kept still says it in its text.
                let parsed = || vcard::parse(&held.vcard).into_iter().next();
                let uid = held
                    .uid
                    .clone()
                    .or_else(|| parsed().and_then(|card| card.uid));
                let (Some(uid), Some(address)) = (uid, held.addresses.first()) else {
                    continue;
                };
                cards.push(Named {
                    key: vcard::uid_key(&uid),
                    name: parsed().and_then(|card| card.display_name()),
                    address: address.clone(),
                });
            }
        }
        Self {
            cards,
            groups: store.groups().unwrap_or_default(),
        }
    }
}

/// Who `group` stands for, nested groups opened and each address once.
pub(in crate::ui) fn expand(store: &dyn Store, group: &Group) -> Expanded {
    let index = Index::of(store);
    let mut out = Expanded::default();
    let mut opened = BTreeSet::from([group.id.clone()]);
    walk(store, &index, group, &mut opened, &mut out);
    out
}

fn walk(
    store: &dyn Store,
    index: &Index,
    group: &Group,
    opened: &mut BTreeSet<GroupId>,
    out: &mut Expanded,
) {
    for uri in &group.members {
        let found = match Member::of(uri) {
            Member::Mailto(addresses) => addresses
                .iter()
                .filter_map(|address| person(store, address, None))
                .map(|person| join(out, person))
                .count(),
            Member::Card(key) => {
                let nested = index
                    .groups
                    .iter()
                    .find(|g| g.uid.as_deref().map(vcard::uid_key) == Some(key.clone()));
                match nested {
                    // Named again inside itself: its members are already on the way in.
                    Some(nested) if !opened.insert(nested.id.clone()) => 1,
                    Some(nested) => {
                        walk(store, index, nested, opened, out);
                        1
                    }
                    None => index
                        .cards
                        .iter()
                        .find(|card| card.key == key)
                        .and_then(|card| person(store, &card.address, card.name.as_deref()))
                        .map(|person| join(out, person))
                        .into_iter()
                        .count(),
                }
            }
        };
        if found == 0 {
            out.unresolved.push(uri.clone());
        }
    }
}

fn join(out: &mut Expanded, person: Person) {
    if !out
        .people
        .iter()
        .any(|p| p.address.eq_ignore_ascii_case(&person.address))
    {
        out.people.push(person);
    }
}

/// The person at `address`, named as the book names them, else as the card does. `None` for
/// something that is not an address.
fn person(store: &dyn Store, address: &str, card_name: Option<&str>) -> Option<Person> {
    let address = mail_store::contact::normalise(address)?;
    Some(match store.contact(&address).ok().flatten() {
        Some(contact) if contact.name.is_some() || card_name.is_none() => person_of(&contact),
        _ => Person {
            name: card_name
                .map(str::to_owned)
                .unwrap_or_else(|| address.split('@').next().unwrap_or(&address).to_owned()),
            address,
        },
    })
}

/// The groups to offer for `typed` in To or Cc: those with a word of their name beginning each
/// word typed, the way the book matches people, up to [`OFFERED`], each expanded. A group that
/// expands to nobody would add nobody, so it is not offered.
pub(in crate::ui) fn offers(store: &dyn Store, typed: &str) -> Vec<Offer> {
    let wanted = search_tokens(typed);
    if wanted.is_empty() {
        return Vec::new();
    }
    let Ok(groups) = store.groups() else {
        return Vec::new();
    };
    groups
        .iter()
        .filter(|group| {
            let words = search_tokens(&group.name);
            wanted
                .iter()
                .all(|want| words.iter().any(|word| word.starts_with(want.as_str())))
        })
        .map(|group| Offer {
            id: group.id.clone(),
            name: group.name.clone(),
            expanded: expand(store, group),
        })
        .filter(|offer| !offer.expanded.people.is_empty())
        .take(OFFERED)
        .collect()
}

/// Every group, with a word of its name beginning each word of `filter`, for the sheet.
pub(in crate::ui) fn listed(store: &dyn Store, filter: &str) -> Result<Vec<Group>, String> {
    let wanted = search_tokens(filter);
    Ok(store
        .groups()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|group| {
            let words = search_tokens(&group.name);
            wanted
                .iter()
                .all(|want| words.iter().any(|word| word.starts_with(want.as_str())))
        })
        .collect())
}

/// A new group of nobody, made here, called `name`.
pub(in crate::ui) fn create(store: &dyn Store, name: &str) -> Result<Group, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A group needs a name.".to_owned());
    }
    let uid = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let group = Group {
        id: GroupId::local(&uid),
        uid: Some(uid),
        name: name.to_owned(),
        members: Vec::new(),
        home: GroupHome::Local,
    };
    store.put_group(&group).map_err(|e| e.to_string())?;
    Ok(group)
}

/// Store `group` as the user changed it. A synced one is marked edited, so the next sync of its
/// book writes it back as a `KIND:group` card.
fn keep(store: &dyn Store, mut group: Group) -> Result<Group, String> {
    if let GroupHome::Book { edit, .. } = &mut group.home {
        *edit = Edit::Edited;
    }
    store.put_group(&group).map_err(|e| e.to_string())?;
    Ok(group)
}

fn load(store: &dyn Store, id: &GroupId) -> Result<Group, String> {
    store
        .group(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "That group is gone.".to_owned())
}

/// Call the group `id` `name`.
pub(in crate::ui) fn rename(store: &dyn Store, id: &GroupId, name: &str) -> Result<Group, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A group needs a name.".to_owned());
    }
    let group = load(store, id)?;
    keep(
        store,
        Group {
            name: name.to_owned(),
            ..group
        },
    )
}

/// Add `typed`, one address or several, to the group `id`, each as a `mailto:` member. An
/// address already in it is not added twice.
pub(in crate::ui) fn add(store: &dyn Store, id: &GroupId, typed: &str) -> Result<Group, String> {
    let addresses = crate::view::parse_addresses(typed)?;
    let mut group = load(store, id)?;
    for address in addresses {
        let Some(address) = mail_store::contact::normalise(&address.email) else {
            continue;
        };
        let already = group.members.iter().any(|uri| {
            matches!(Member::of(uri), Member::Mailto(named)
                if named.iter().any(|n| n.eq_ignore_ascii_case(&address)))
        });
        if !already {
            group.members.push(format!("mailto:{address}"));
        }
    }
    keep(store, group)
}

/// Take the member written `uri` out of the group `id`.
pub(in crate::ui) fn remove(store: &dyn Store, id: &GroupId, uri: &str) -> Result<Group, String> {
    let mut group = load(store, id)?;
    group.members.retain(|member| member != uri);
    keep(store, group)
}

/// Forget a group made here. A synced group is its address book's to delete.
pub(in crate::ui) fn forget(store: &dyn Store, id: &GroupId) -> Result<(), String> {
    let group = load(store, id)?;
    if group.home != GroupHome::Local {
        return Err(format!(
            "{} is in an address book: delete it there, and the next sync takes it away here.",
            group.name
        ));
    }
    store.delete_group(id).map_err(|e| e.to_string())?;
    Ok(())
}

/// Each member of `group` as written, with what the sheet shows for it: the person it names,
/// or the URI when it names nobody known.
pub(in crate::ui) fn member_labels(store: &dyn Store, group: &Group) -> Vec<(String, String)> {
    let index = Index::of(store);
    group
        .members
        .iter()
        .map(|uri| {
            let one = Group {
                members: vec![uri.clone()],
                ..group.clone()
            };
            let mut out = Expanded::default();
            walk(
                store,
                &index,
                &one,
                &mut BTreeSet::from([group.id.clone()]),
                &mut out,
            );
            let label = match out.people.as_slice() {
                [] => format!("{uri} (not found)"),
                [person] if person.name != person.address => {
                    format!("{} <{}>", person.name, person.address)
                }
                people => people
                    .iter()
                    .map(|p| p.address.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            (uri.clone(), label)
        })
        .collect()
}
