//! The Folders section: an account's own mailboxes, nested, and what can be done to them.
//!
//! Each folder is quire's `TreeItem`: a parent opens and closes as the row asks (the row keeps
//! whether it is open), its name chooses it, and its ⋯ opens quire's menu beside it. A new
//! folder is named in a [`Field`] under the row it goes in; a rename is written in the name's
//! own place, quire's `TreeItem { editing }`.

use super::super::menu::{Floating, MenuItem};
use super::folder_parts::{Naming, Said, item};
use super::folder_row::FolderRow;
use super::folder_tree::{Section, Show};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{Icon, MenuKind, MountedRef};
use mail_domain::AccountId;

/// One folder, by account and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Spot {
    pub account: AccountId,
    pub path: String,
}

/// What the section has open. One thing at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Open {
    Closed,
    /// The ⋯ menu on one folder.
    Actions(Spot),
    /// Which account a new top-level folder goes on, when the scope holds several.
    Accounts,
    /// Naming a new folder inside `parent`, or at the top.
    Naming {
        account: AccountId,
        parent: Option<String>,
        text: String,
    },
    Renaming {
        spot: Spot,
        text: String,
    },
    /// A delete was asked of a folder that holds mail: the menu asks whether it goes too.
    Confirm {
        spot: Spot,
        messages: u64,
    },
}

/// A refusal, said under the row it happened on (`path: None` is the header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Note {
    pub account: AccountId,
    pub path: Option<String>,
    pub text: String,
}

/// Focus the name field once it is drawn, and select what is in it.
pub(super) fn focus_name() {
    crate::ui::host::Host::focus_and_select(crate::ui::host::Drawn::FolderName);
}

/// The props every row passes down, because a row draws its children.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Wires {
    pub shell: Signal<Shell>,
    pub pages: Signal<u32>,
    pub badges: Memo<Vec<Option<u64>>>,
    pub revision: Signal<u64>,
    pub open: Signal<Open>,
    pub note: Signal<Option<Note>>,
}

#[component]
pub(super) fn FolderList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    revision: Signal<u64>,
    section: Section,
    show: Signal<Show>,
) -> Element {
    let mut open = use_signal(|| Open::Closed);
    let note = use_signal(|| None::<Note>);
    let wires = Wires {
        shell,
        pages,
        badges,
        revision,
        open,
        note,
    };
    let trees = section.trees.clone();
    let several = trees.len() > 1;
    let empty = trees.iter().all(|tree| tree.nodes.is_empty());
    let accounts: Vec<(AccountId, String, Option<char>)> = trees
        .iter()
        .map(|tree| (tree.account, tree.address.clone(), tree.delimiter))
        .collect();
    let first = accounts.first().map(|(id, _, _)| *id);
    let toggle = if show() == Show::All {
        "Followed only"
    } else {
        "Show all"
    };
    let delimiter_of = move |account: AccountId| {
        accounts
            .iter()
            .find(|(id, _, _)| *id == account)
            .and_then(|(_, _, d)| *d)
    };
    let pick_items: Vec<MenuItem> = trees
        .iter()
        .map(|tree| item(&tree.account.to_string(), Icon::Plus, &tree.address, None))
        .collect();
    let mut new_button = use_signal(|| None::<MountedRef>);
    let hint = if show() == Show::All {
        "Hide the folders you do not follow"
    } else {
        "Show the folders you do not follow"
    };
    rsx! {
        div { class: "s-h",
            "Folders"
            if section.hidden > 0 || show() == Show::All {
                ds::Button {
                    variant: ds::ButtonVariant::Frame,
                    label: toggle,
                    title: hint.to_owned(),
                    onclick: move |_: ds::Press| {
                        let mut show = show;
                        show.set(if show() == Show::All { Show::Followed } else { Show::All });
                    },
                }
            }
            ds::Button {
                variant: ds::ButtonVariant::Frame,
                label: "+ New",
                aria_label: "New folder".to_owned(),
                title: "New folder".to_owned(),
                mounted: move |event: MountedEvent| new_button.set(Some(MountedRef(event.data()))),
                onclick: move |_: ds::Press| {
                    if several {
                        open.set(Open::Accounts);
                    } else if let Some(account) = first {
                        open.set(Open::Naming { account, parent: None, text: String::new() });
                        focus_name();
                    }
                },
            }
        }
        if *open.read() == Open::Accounts {
            Floating {
                kind: MenuKind::Slim,
                anchor: new_button(),
                title: "New folder on".to_owned(),
                items: pick_items,
                on_pick: move |key: String| {
                    if let Ok(uuid) = key.parse() {
                        let account = AccountId::from_uuid(uuid);
                        open.set(Open::Naming { account, parent: None, text: String::new() });
                        focus_name();
                    }
                },
                // The pick opened the name field: closing the menu must leave it open.
                on_close: move |_| {
                    if *open.peek() == Open::Accounts {
                        open.set(Open::Closed);
                    }
                },
            }
        }
        Naming { wires, at: None, delimiter_of: Callback::new(delimiter_of) }
        Said { wires, account: None, path: None }
        if empty {
            div { class: "fold-note", "No folders of your own yet." }
        }
        for tree in trees {
            if several {
                div { key: "a-{tree.account}", class: "fold-acct", "{tree.address}" }
            }
            for node in tree.nodes {
                FolderRow {
                    key: "{tree.account}-{node.path}",
                    node,
                    account: tree.account,
                    delimiter: tree.delimiter,
                    wires,
                }
            }
        }
    }
}
