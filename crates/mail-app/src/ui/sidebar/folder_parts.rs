//! The small pieces the rows share: the name fields, a refusal in words, and the menu's items.

use super::super::menu::{MenuItem, Right, Tile};
use super::folder_act::child_path;
use super::folder_row::run;
use super::folder_tree::{Kind, Node};
use super::folders::{Note, Open, Spot, Wires};
use dioxus::prelude::*;
use ds::components::content::label::LabelRole;
use ds::components::lists::row::row::Outline;
use ds::focus::request::use_focus_request;
use ds::prelude::*;
use mail_domain::{FolderWork, Subscription};
use porter_core::AccountId;

/// The row for a new folder's name, under the folder it goes in (or at the top of the list):
/// quire's `Row`, its words the name field.
#[component]
pub(super) fn Naming(
    wires: Wires,
    at: Option<Spot>,
    delimiter_of: Callback<AccountId, Option<char>>,
) -> Element {
    let mut open = wires.open;
    let mut note = wires.note;
    let (account, parent, text) = match &*open.read() {
        Open::Naming {
            account,
            parent,
            text,
        } => (account.clone(), parent.clone(), text.clone()),
        _ => return rsx! {},
    };
    let here = match &at {
        Some(spot) => spot.account == account && parent.as_deref() == Some(spot.path.as_str()),
        None => parent.is_none(),
    };
    if !here {
        return rsx! {};
    }
    let delimiter = delimiter_of.call(account);
    rsx! {
        Row {
            title: "New folder",
            outline: Outline::Leaf,
            edit: rsx! {
                NameField {
                    value: text,
                    placeholder: "New folder".to_owned(),
                    on_input: move |text: String| {
                        let now = open.peek().clone();
                        if let Open::Naming { account, parent, .. } = now {
                            open.set(Open::Naming { account, parent, text });
                        }
                    },
                    on_commit: move |_| {
                        let Open::Naming { account, parent, text } = open.peek().clone() else { return };
                        match child_path(parent.as_deref(), &text, delimiter) {
                            Ok(path) => run(wires, account, parent, FolderWork::Create { path }, delimiter),
                            Err(text) => note.set(Some(Note { account, path: parent, text })),
                        }
                    },
                    on_cancel: move |_| open.set(Open::Closed),
                }
            },
        }
    }
}

/// A refusal, when it belongs here: the row's secondary line, said as an alert to a screen
/// reader.
#[component]
pub(super) fn Said(wires: Wires, account: Option<AccountId>, path: Option<String>) -> Element {
    let note = wires.note.read().clone();
    match note {
        Some(note) if note.path == path && account.is_none_or(|a| a == note.account) => rsx! {
            Label { text: note.text.clone(), role: LabelRole::Secondary }
        },
        _ => rsx! {},
    }
}

/// A folder's name, written where the row says it: quire's `TextField` without its bezel, with
/// the keyboard as it mounts and the old name selected. Enter makes it so and Esc leaves it;
/// the row takes the field away to end it.
#[component]
pub(super) fn NameField(
    value: String,
    placeholder: String,
    on_input: EventHandler<String>,
    on_commit: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    let focus = FieldFocus::Controlled(use_focus_request().with_select_all());
    rsx! {
        TextField {
            label: "Folder name".to_owned(),
            value,
            placeholder,
            bezel: FieldBezel::Plain,
            focus,
            oninput: move |text: String| on_input.call(text),
            onkey: move |event: KeyboardEvent| {
                // Ctrl chords are still the window's. Every other key is the field's: a letter
                // typed into a name is not a shortcut.
                if event.modifiers().ctrl() || event.modifiers().meta() {
                    return;
                }
                event.stop_propagation();
                match event.key().to_string().as_str() {
                    "Enter" => on_commit.call(()),
                    "Escape" => on_cancel.call(()),
                    _ => {}
                }
            },
        }
    }
}

/// The ⋯ menu's items for `node`.
pub(super) fn actions(node: &Node) -> Vec<MenuItem> {
    let new = item("new", Icon::Plus, "New folder inside", None);
    match node.kind {
        Kind::Implied => vec![new],
        Kind::Listed { subscription, .. } => {
            let follow = match subscription {
                Subscription::Subscribed => item(
                    "unfollow",
                    Icon::X,
                    "Stop following",
                    Some("Other mail apps may stop showing it"),
                ),
                Subscription::Unsubscribed => item("follow", Icon::Check, "Follow", None),
            };
            vec![
                new,
                item("rename", Icon::Pen, "Rename", None),
                follow,
                item("delete", Icon::Trash, "Delete", None),
            ]
        }
    }
}

pub(super) fn item(key: &str, icon: Icon, name: &str, help: Option<&str>) -> MenuItem {
    MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: help.map(str::to_owned),
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}
