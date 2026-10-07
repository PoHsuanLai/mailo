//! The Contacts page's groups, under the people, each a row of quire's list: one open at a time
//! to rename, to add an address to, or to take a member out of; a new one made from a name.
//!
//! Every write is `groups`', on the event that asks for it. An edit to a synced group is marked
//! for the next `mailo contacts sync`, which writes it back to its address book, and the row
//! says so until then.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::{Edit, Group, GroupHome, GroupId, SqliteStore};

use super::super::common::{classed, tile};
use super::super::press::on_primary;
use super::group_edit::{AddRow, MemberRow, RenameRow};
use super::groups;
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Answers, ButtonRole};
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use ds::style::tokens::control_size::ControlSize;

/// The group open for editing, and what is typed in its two fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Open {
    pub(super) id: GroupId,
    pub(super) name: String,
    pub(super) adding: String,
}

/// The groups whose names fit `filter`, the open one's members and fields under it, and the way
/// to make one: every one a row of quire's list.
#[component]
pub(super) fn GroupRows(
    filter: String,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let open = use_signal(|| None::<Open>);
    let making = use_signal(|| None::<String>);
    // Read on every render, which `changed` asks for after each write: a book holds a handful
    // of groups, where it may hold thousands of people.
    let _ = changed();
    let listed = {
        let store = consume_context::<Arc<SqliteStore>>();
        groups::listed(store.as_ref(), &filter).unwrap_or_default()
    };
    let mut items: Vec<ListItem<String>> = Vec::new();
    if listed.is_empty() {
        items.push(ListItem::row(
            "none".to_owned(),
            "No groups".to_owned(),
            rsx! { Row { title: TextLine::Runs(vec![TextRun::new("No groups yet", RunTone::Faint)]), size: RowSize::Settings } },
        ));
    }
    for group in listed {
        let editing = open.read().as_ref().filter(|o| o.id == group.id).cloned();
        let id = group.id.to_string();
        if let Some(editing) = editing {
            let labels = {
                let store = consume_context::<Arc<SqliteStore>>();
                groups::member_labels(store.as_ref(), &group)
            };
            items.push(ListItem::row(
                format!("g:{id}"),
                group.name.clone(),
                rsx! { GroupRow { group: group.clone(), open, changed, said } },
            ));
            items.push(ListItem::row(
                format!("n:{id}"),
                "Name".to_owned(),
                rsx! { RenameRow { id: group.id.clone(), typed: editing.name.clone(), open, changed, said } },
            ));
            for (uri, label) in labels {
                items.push(ListItem::row(
                    format!("m:{id}:{uri}"),
                    label.clone(),
                    rsx! { MemberRow { id: group.id.clone(), uri, label, changed, said } },
                ));
            }
            items.push(ListItem::row(
                format!("a:{id}"),
                "Add an address".to_owned(),
                rsx! { AddRow { id: group.id.clone(), typed: editing.adding.clone(), open, changed, said } },
            ));
        } else {
            items.push(ListItem::row(
                format!("g:{id}"),
                group.name.clone(),
                rsx! { GroupRow { group, open, changed, said } },
            ));
        }
    }
    items.push(ListItem::row(
        "new".to_owned(),
        "New group".to_owned(),
        rsx! { NewGroupRow { making, changed, said } },
    ));
    rsx! {
        FormSection { title: Some("Groups".to_owned()),
            List::<String> { label: "Groups".to_owned(), items, style: ListStyle::Grouped }
        }
    }
}

/// The last row: New Group, or the new group's name being typed, with Make.
#[component]
fn NewGroupRow(
    making: Signal<Option<String>>,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let make = move || {
        let name = making.peek().clone().unwrap_or_default();
        let store = consume_context::<Arc<SqliteStore>>();
        match groups::create(store.as_ref(), &name) {
            Ok(group) => {
                said.set(Some(Ok(format!("Made the group {}.", group.name))));
                making.set(None);
                changed += 1;
            }
            Err(why) => said.set(Some(Err(why))),
        }
    };
    let mut make_on_enter = make;
    let Some(typed) = making() else {
        return rsx! {
            Row {
                leading: tile(Icon::Plus, PlateFamily::Green),
                title: "New group",
                size: RowSize::Settings,
                accessory: Accessory::Slot(rsx! {
                    Button {
                        size: ControlSize::Small,
                        label: "New Group…".to_owned(),
                        common: Common { aria_label: Some("New group".to_owned()), ..Common::default() },
                        onclick: on_primary(move || making.set(Some(String::new()))),
                    }
                }),
            }
        };
    };
    rsx! {
        Row {
            leading: tile(Icon::Plus, PlateFamily::Green),
            title: "New group",
            size: RowSize::Settings,
            content: rsx! {
                div {
                    class: "naming",
                    onkeydown: move |event: KeyboardEvent| match event.key().to_string().as_str() {
                        "Enter" => {
                            event.prevent_default();
                            make_on_enter();
                        }
                        "Escape" => {
                            event.stop_propagation();
                            making.set(None);
                        }
                        _ => {}
                    },
                    TextField {
                        label: "The group's name".to_owned(),
                        value: typed,
                        placeholder: "The group's name".to_owned(),
                        focus: FieldFocus::OnMount,
                        common: classed("book-name"),
                        oninput: move |value: String| making.set(Some(value)),
                    }
                }
            },
            accessory: Accessory::Slot(rsx! {
                Button {
                    size: ControlSize::Small,
                    label: "Cancel".to_owned(),
                    common: Common { aria_label: Some("Cancel the new group".to_owned()), ..Common::default() },
                    onclick: on_primary(move || making.set(None)),
                }
                Button {
                    size: ControlSize::Small,
                    answers: Answers::Return,
                    label: "Make".to_owned(),
                    common: Common { aria_label: Some("Make the group".to_owned()), ..Common::default() },
                    onclick: on_primary(make),
                }
            }),
        }
    }
}

/// Where a group lives, and whether an edit waits for a sync, in a few words.
fn standing(group: &Group) -> &'static str {
    match group.home {
        GroupHome::Local => "made here",
        GroupHome::Book {
            edit: Edit::Synced, ..
        } => "from an address book",
        GroupHome::Book {
            edit: Edit::Edited, ..
        } => "edited · written back by mailo contacts sync",
    }
}

/// One group: its name, how many it holds and where it lives, Edit (Done while open) and, for one
/// made here, Delete.
#[component]
fn GroupRow(
    group: Group,
    open: Signal<Option<Open>>,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let count = match group.members.len() {
        0 => "nobody yet".to_owned(),
        1 => "1 member".to_owned(),
        n => format!("{n} members"),
    };
    let editing = open.read().as_ref().is_some_and(|o| o.id == group.id);
    let id = group.id.clone();
    let name = group.name.clone();
    let local = group.home == GroupHome::Local;
    let gone = id.clone();
    rsx! {
        Row {
            leading: tile(Icon::Group, PlateFamily::Violet),
            title: group.name.clone(),
            detail: Some(TextLine::from(format!("{count} · {}", standing(&group)))),
            size: RowSize::Settings,
            accessory: Accessory::Slot(rsx! {
                Button {
                    size: ControlSize::Small,
                    label: if editing { "Done".to_owned() } else { "Edit".to_owned() },
                    common: Common {
                        aria_label: Some(format!("Edit the group {name}")),
                        ..Common::default()
                    },
                    onclick: on_primary(move || {
                        let shut = open.peek().as_ref().is_some_and(|o| o.id == id);
                        open.set((!shut).then(|| Open {
                            id: id.clone(),
                            name: name.clone(),
                            adding: String::new(),
                        }));
                    }),
                }
                if local {
                    Button {
                        size: ControlSize::Small,
                        role: ButtonRole::Destructive,
                        label: "Delete".to_owned(),
                        common: Common {
                            aria_label: Some(format!("Delete the group {}", group.name)),
                            ..Common::default()
                        },
                        onclick: on_primary(move || {
                            let store = consume_context::<Arc<SqliteStore>>();
                            said.set(Some(
                                groups::forget(store.as_ref(), &gone)
                                    .map(|()| "Deleted the group. Its people stay in the book.".to_owned()),
                            ));
                            open.set(None);
                            changed += 1;
                        }),
                    }
                }
            }),
        }
    }
}
