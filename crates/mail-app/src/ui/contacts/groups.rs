//! Contact groups in the window, as functions of a store: who a group is, which groups to offer
//! for what was typed in To, and the edits a person makes in the Contacts sheet.
//!
//! The groups themselves are `mail_core::contacts::groups`; this words their refusals for the
//! sheet and labels a group's members.

use mail_core::Store;
use mail_core::contacts::groups::{self as shared, GroupError, Labelled};
use mail_core::contacts::{Group, GroupId};

pub(in crate::ui) use mail_core::contacts::groups::{Expanded, OFFERED, Offer, expand, offers};

/// Why a change to a group was not made, as the sheet says it.
fn refused(why: GroupError) -> String {
    match why {
        GroupError::NoName => "A group needs a name.".to_owned(),
        GroupError::Gone => "That group is gone.".to_owned(),
        GroupError::InBook { name } => format!(
            "{name} is in an address book: delete it there, and the next sync takes it away here."
        ),
        GroupError::Core(why) => why.to_string(),
    }
}

/// Every group, with a word of its name beginning each word of `filter`, for the sheet.
pub(in crate::ui) fn listed(store: &dyn Store, filter: &str) -> Result<Vec<Group>, String> {
    shared::listed(store, filter).map_err(|e| e.to_string())
}

/// A new group of nobody, made here, called `name`.
pub(in crate::ui) fn create(store: &dyn Store, name: &str) -> Result<Group, String> {
    shared::create(store, name).map_err(refused)
}

/// Call the group `id` `name`.
pub(in crate::ui) fn rename(store: &dyn Store, id: &GroupId, name: &str) -> Result<Group, String> {
    shared::rename(store, id, name).map_err(refused)
}

/// Add `typed`, one address or several, to the group `id`, each as a `mailto:` member. An
/// address already in it is not added twice.
pub(in crate::ui) fn add(store: &dyn Store, id: &GroupId, typed: &str) -> Result<Group, String> {
    shared::add(store, id, typed).map_err(refused)
}

/// Take the member written `uri` out of the group `id`.
pub(in crate::ui) fn remove(store: &dyn Store, id: &GroupId, uri: &str) -> Result<Group, String> {
    shared::remove(store, id, uri).map_err(refused)
}

/// Forget a group made here. A synced group is its address book's to delete.
pub(in crate::ui) fn forget(store: &dyn Store, id: &GroupId) -> Result<(), String> {
    shared::forget(store, id).map_err(refused)
}

/// Each member of `group` as written, with what the sheet shows for it: the person it names,
/// or the URI when it names nobody known.
pub(in crate::ui) fn member_labels(store: &dyn Store, group: &Group) -> Vec<(String, String)> {
    shared::members(store, group)
        .into_iter()
        .map(|(uri, labelled)| {
            let label = match labelled {
                Labelled::NotFound => format!("{uri} (not found)"),
                Labelled::Named(person) => {
                    mail_domain::Address::named(&person.name, &person.address).to_string()
                }
                Labelled::Addresses(addresses) => addresses.join(", "),
            };
            (uri, label)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_change_is_said_for_the_sheet() {
        assert_eq!(refused(GroupError::NoName), "A group needs a name.");
        assert_eq!(refused(GroupError::Gone), "That group is gone.");
        assert_eq!(
            refused(GroupError::InBook {
                name: "Team".to_owned()
            }),
            "Team is in an address book: delete it there, and the next sync takes it away here."
        );
    }
}
