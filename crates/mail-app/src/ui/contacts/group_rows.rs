//! The Contacts sheet's groups: listed above the people, one open at a time to rename, to add
//! an address to, or to take a member out of; a new one made from a name.
//!
//! Every write is `groups`', on the event that asks for it. An edit to a synced group is marked
//! for the next `mailo contacts sync`, which writes it back to its address book, and the row
//! says so until then.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::{Edit, Group, GroupHome, GroupId, SqliteStore};

use super::super::field::{Field, FieldKind};
use super::super::press::on_primary;
use super::groups;
use ds::{Glyph, Icon};

/// The group open for editing, and what is typed in its two fields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Open {
    id: GroupId,
    name: String,
    adding: String,
}

/// The groups whose names fit `filter`, and the way to make one.
#[component]
pub(super) fn GroupRows(
    filter: String,
    changed: Signal<u64>,
    said: Signal<Option<String>>,
) -> Element {
    let open = use_signal(|| None::<Open>);
    let mut making = use_signal(|| None::<String>);
    // Read on every render, which `changed` asks for after each write: a book holds a handful
    // of groups, where it may hold thousands of people.
    let _ = changed();
    let listed = {
        let store = consume_context::<Arc<SqliteStore>>();
        groups::listed(store.as_ref(), &filter).unwrap_or_default()
    };
    let make = move || {
        let name = making.peek().clone().unwrap_or_default();
        let store = consume_context::<Arc<SqliteStore>>();
        match groups::create(store.as_ref(), &name) {
            Ok(group) => {
                said.set(Some(format!("Made the group {}.", group.name)));
                making.set(None);
                changed += 1;
            }
            Err(why) => said.set(Some(why)),
        }
    };
    let mut make_on_enter = make;
    rsx! {
        li { class: "book-groups",
            div { class: "book-sub",
                h4 { "Groups" }
                ds::Button {
                    variant: ds::ButtonVariant::Mini,
                    label: "New group".to_owned(),
                    aria_label: "New group".to_owned(),
                    icon: Icon::Plus,
                    onclick: on_primary(move || making.set(Some(String::new()))),
                }
            }
            if let Some(typed) = making() {
                div {
                    class: "naming group-new",
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
                    Field {
                        kind: FieldKind::Boxed,
                        value: typed,
                        placeholder: "The group's name".to_owned(),
                        extra: Some("book-name".to_owned()),
                        on_input: move |value: String| making.set(Some(value)),
                        on_focus: |_| {},
                        on_blur: |_| {},
                    }
                    ds::Button {
                        variant: ds::ButtonVariant::Primary,
                        label: "Make".to_owned(),
                        aria_label: "Make the group".to_owned(),
                        onclick: on_primary(make),
                    }
                }
            }
            ul { class: "group-list",
                for group in listed {
                    GroupRow { key: "{group.id}", group, open, changed, said }
                }
            }
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

/// One group: its name and how many it holds, and when open, its members and the fields that
/// change it.
#[component]
fn GroupRow(
    group: Group,
    open: Signal<Option<Open>>,
    changed: Signal<u64>,
    said: Signal<Option<String>>,
) -> Element {
    let count = match group.members.len() {
        0 => "nobody yet".to_owned(),
        1 => "1 member".to_owned(),
        n => format!("{n} members"),
    };
    let editing = open.read().as_ref().filter(|o| o.id == group.id).cloned();
    let id = group.id.clone();
    let name = group.name.clone();
    let local = group.home == GroupHome::Local;
    let labels: Vec<(String, String)> = if editing.is_some() {
        let store = consume_context::<Arc<SqliteStore>>();
        groups::member_labels(store.as_ref(), &group)
    } else {
        Vec::new()
    };
    rsx! {
        li { class: "book-row group-row",
            span { class: "av group-av", Glyph { icon: Icon::Group } }
            div { class: "who",
                b { "{group.name}" }
                span { class: "origin", "{count} · {standing(&group)}" }
            }
            div { class: "acts",
                ds::Button {
                    variant: ds::ButtonVariant::Secondary,
                    label: if editing.is_some() { "Done".to_owned() } else { "Edit".to_owned() },
                    aria_label: format!("Edit the group {name}"),
                    onclick: {
                        let id = id.clone();
                        let name = name.clone();
                        on_primary(move || {
                            let shut = open.peek().as_ref().is_some_and(|o| o.id == id);
                            open.set((!shut).then(|| Open {
                                id: id.clone(),
                                name: name.clone(),
                                adding: String::new(),
                            }));
                        })
                    },
                }
                if local {
                    ds::Button {
                        variant: ds::ButtonVariant::Danger,
                        label: "Delete".to_owned(),
                        aria_label: format!("Delete the group {name}"),
                        onclick: {
                            let id = id.clone();
                            on_primary(move || {
                                let store = consume_context::<Arc<SqliteStore>>();
                                said.set(Some(match groups::forget(store.as_ref(), &id) {
                                    Ok(()) => "Deleted the group. Its people stay in the book.".to_owned(),
                                    Err(why) => why,
                                }));
                                open.set(None);
                                changed += 1;
                            })
                        },
                    }
                }
            }
            if let Some(editing) = editing {
                GroupEditor { id: id.clone(), editing, labels, open, changed, said }
            }
        }
    }
}

/// The open group's fields: its name, its members each with a way out, and an address to add.
#[component]
fn GroupEditor(
    id: GroupId,
    editing: Open,
    labels: Vec<(String, String)>,
    open: Signal<Option<Open>>,
    changed: Signal<u64>,
    said: Signal<Option<String>>,
) -> Element {
    let rename = {
        let id = id.clone();
        move || {
            let typed = open
                .peek()
                .as_ref()
                .map(|o| o.name.clone())
                .unwrap_or_default();
            let store = consume_context::<Arc<SqliteStore>>();
            if let Err(why) = groups::rename(store.as_ref(), &id, &typed) {
                said.set(Some(why));
            }
            changed += 1;
        }
    };
    let add = {
        let id = id.clone();
        move || {
            let typed = open
                .peek()
                .as_ref()
                .map(|o| o.adding.clone())
                .unwrap_or_default();
            let store = consume_context::<Arc<SqliteStore>>();
            match groups::add(store.as_ref(), &id, &typed) {
                Ok(_) => {
                    if let Some(o) = open.write().as_mut() {
                        o.adding.clear();
                    }
                }
                Err(why) => said.set(Some(why)),
            }
            changed += 1;
        }
    };
    let mut rename_on_enter = rename.clone();
    let mut add_on_enter = add.clone();
    rsx! {
        div { class: "group-edit",
            div {
                class: "naming",
                onkeydown: move |event: KeyboardEvent| {
                    if event.key().to_string() == "Enter" {
                        event.prevent_default();
                        rename_on_enter();
                    }
                },
                Field {
                    kind: FieldKind::Boxed,
                    value: editing.name.clone(),
                    placeholder: "The group's name".to_owned(),
                    extra: Some("book-name group-name".to_owned()),
                    on_input: move |value: String| {
                        if let Some(o) = open.write().as_mut() {
                            o.name = value;
                        }
                    },
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
                ds::Button {
                    variant: ds::ButtonVariant::Secondary,
                    label: "Rename".to_owned(),
                    aria_label: "Rename the group".to_owned(),
                    onclick: on_primary(rename),
                }
            }
            ul { class: "group-members",
                for (uri, label) in labels {
                    li { key: "{uri}", class: "group-member",
                        span { class: "addr", "{label}" }
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: "Remove".to_owned(),
                            aria_label: format!("Remove {label}"),
                            onclick: {
                                let id = id.clone();
                                on_primary(move || {
                                    let store = consume_context::<Arc<SqliteStore>>();
                                    if let Err(why) = groups::remove(store.as_ref(), &id, &uri) {
                                        said.set(Some(why));
                                    }
                                    changed += 1;
                                })
                            },
                        }
                    }
                }
            }
            div {
                class: "naming",
                onkeydown: move |event: KeyboardEvent| {
                    if event.key().to_string() == "Enter" {
                        event.prevent_default();
                        add_on_enter();
                    }
                },
                Field {
                    kind: FieldKind::Boxed,
                    value: editing.adding.clone(),
                    placeholder: "Add an address".to_owned(),
                    extra: Some("book-name group-add".to_owned()),
                    on_input: move |value: String| {
                        if let Some(o) = open.write().as_mut() {
                            o.adding = value;
                        }
                    },
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
                ds::Button {
                    variant: ds::ButtonVariant::Primary,
                    label: "Add".to_owned(),
                    aria_label: "Add to the group".to_owned(),
                    onclick: on_primary(add),
                }
            }
        }
    }
}
