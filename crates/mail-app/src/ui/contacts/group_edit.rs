//! The open group's own rows on the Contacts page, under its row: its name in a field, each
//! member with Remove, and an address to add.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::{GroupId, SqliteStore};

use super::super::common::{classed, tile};
use super::super::press::on_primary;
use super::group_rows::Open;
use super::groups;
use ds::components::controls::button_model::Answers;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use ds::style::tokens::control_size::ControlSize;

/// The open group's name, in a field, with Rename.
#[component]
pub(super) fn RenameRow(
    id: GroupId,
    typed: String,
    open: Signal<Option<Open>>,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let rename = move || {
        let typed = open
            .peek()
            .as_ref()
            .map(|o| o.name.clone())
            .unwrap_or_default();
        let store = consume_context::<Arc<SqliteStore>>();
        if let Err(why) = groups::rename(store.as_ref(), &id, &typed) {
            said.set(Some(Err(why)));
        }
        changed += 1;
    };
    let mut rename_on_enter = rename.clone();
    rsx! {
        Row {
            leading: tile(Icon::Pen, PlateFamily::Amber),
            title: "Name",
            size: RowSize::Settings,
            content: rsx! {
                div {
                    class: "naming",
                    onkeydown: move |event: KeyboardEvent| {
                        if event.key().to_string() == "Enter" {
                            event.prevent_default();
                            rename_on_enter();
                        }
                    },
                    TextField {
                        label: "The group's name".to_owned(),
                        value: typed,
                        placeholder: "The group's name".to_owned(),
                        common: classed("book-name"),
                        oninput: move |value: String| {
                            if let Some(o) = open.write().as_mut() {
                                o.name = value;
                            }
                        },
                    }
                }
            },
            accessory: Accessory::Slot(rsx! {
                Button {
                    size: ControlSize::Small,
                    label: "Rename".to_owned(),
                    common: Common {
                        aria_label: Some("Rename the group".to_owned()),
                        ..Common::default()
                    },
                    onclick: on_primary(rename),
                }
            }),
        }
    }
}

/// One member of the open group, with Remove.
#[component]
pub(super) fn MemberRow(
    id: GroupId,
    uri: String,
    label: String,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    rsx! {
        Row {
            leading: tile(Icon::Mail, PlateFamily::Blue),
            title: label.clone(),
            size: RowSize::Settings,
            accessory: Accessory::Slot(rsx! {
                Button {
                    size: ControlSize::Small,
                    label: "Remove".to_owned(),
                    common: Common {
                        aria_label: Some(format!("Remove {label}")),
                        ..Common::default()
                    },
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        if let Err(why) = groups::remove(store.as_ref(), &id, &uri) {
                            said.set(Some(Err(why)));
                        }
                        changed += 1;
                    }),
                }
            }),
        }
    }
}

/// An address to add to the open group, in a field, with Add.
#[component]
pub(super) fn AddRow(
    id: GroupId,
    typed: String,
    open: Signal<Option<Open>>,
    changed: Signal<u64>,
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let add = move || {
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
            Err(why) => said.set(Some(Err(why))),
        }
        changed += 1;
    };
    let mut add_on_enter = add.clone();
    rsx! {
        Row {
            leading: tile(Icon::Plus, PlateFamily::Green),
            title: "Add an address",
            size: RowSize::Settings,
            content: rsx! {
                div {
                    class: "naming",
                    onkeydown: move |event: KeyboardEvent| {
                        if event.key().to_string() == "Enter" {
                            event.prevent_default();
                            add_on_enter();
                        }
                    },
                    TextField {
                        label: "Add an address".to_owned(),
                        value: typed,
                        placeholder: "Add an address".to_owned(),
                        common: classed("book-name"),
                        oninput: move |value: String| {
                            if let Some(o) = open.write().as_mut() {
                                o.adding = value;
                            }
                        },
                    }
                }
            },
            accessory: Accessory::Slot(rsx! {
                Button {
                    size: ControlSize::Small,
                    answers: Answers::Return,
                    label: "Add".to_owned(),
                    common: Common {
                        aria_label: Some("Add to the group".to_owned()),
                        ..Common::default()
                    },
                    onclick: on_primary(add),
                }
            }),
        }
    }
}
