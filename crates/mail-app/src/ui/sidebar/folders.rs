//! The Folders section: an account's own mailboxes, nested, and what can be done to them.
//!
//! Each folder is quire's `Row` in a source list: a parent is an outline branch that opens and
//! closes as its triangle asks, its name chooses it, and its ⋯ or a right-click opens quire's
//! menu. A new folder is named in a row of its own under the folder it goes in; a rename is
//! written in the name's own place.

use super::super::menu::{Floating, MenuItem};
use super::folder_parts::{Naming, Said, item};
use super::folder_row::FolderRow;
use super::folder_tree::{Section, Show};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::components::lists::row::row::Outline;
use ds::components::lists::section_header::HeaderAction;
use ds::host::measure::MountedRef;
use ds::prelude::*;
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

/// What a folder list's items are keyed by: an account's heading, the field that names a new
/// top-level folder, a folder by its account and path, or the toggle at the foot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FolderKey {
    Account(AccountId),
    New,
    Folder(AccountId, String),
    Toggle,
    Empty,
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
    let toggling = section.hidden > 0 || show() == Show::All;
    let mut items = Vec::new();
    if matches!(&*open.read(), Open::Naming { parent: None, .. }) {
        items.push(ListItem::row(
            FolderKey::New,
            "New folder",
            rsx! {
                Naming { wires, at: None, delimiter_of: Callback::new(delimiter_of) }
            },
        ));
    }
    if empty {
        items.push(
            ListItem::row(
                FolderKey::Empty,
                "No folders",
                rsx! {
                    Label {
                        text: "No folders of your own yet.",
                        role: ds::components::content::label::LabelRole::Tertiary,
                    }
                },
            )
            .with(Availability::Disabled),
        );
    }
    for tree in trees {
        if several {
            items.push(ListItem::heading(
                FolderKey::Account(tree.account),
                rsx! { SectionHeader { title: tree.address.clone() } },
            ));
        }
        for node in tree.nodes {
            let key = FolderKey::Folder(tree.account, node.path.clone());
            let label = node.name.clone();
            let account = tree.account;
            let delimiter = tree.delimiter;
            items.push(ListItem::row(
                key,
                label,
                rsx! {
                    FolderRow { node, account, delimiter, wires }
                },
            ));
        }
    }
    if toggling {
        let all = show() == Show::All;
        items.push(ListItem::row(
            FolderKey::Toggle,
            "Show all folders",
            rsx! {
                Row {
                    leading: RowLeading::Icon(Icon::Folder),
                    title: if all { "Followed only" } else { "Show all" },
                    outline: Outline::None,
                    common: super::tagged("hint", if all {
                        "Hide the folders you do not follow"
                    } else {
                        "Show the folders you do not follow"
                    }),
                    onclick: move |_| {
                        let mut show = show;
                        show.set(if show() == Show::All { Show::Followed } else { Show::All });
                    },
                }
            },
        ));
    }
    rsx! {
        SectionHeader {
            title: "Folders",
            actions: vec![HeaderAction {
                onmounted: Some(EventHandler::new(move |event: MountedEvent| {
                    new_button.set(Some(MountedRef(event.data())));
                })),
                ..HeaderAction::new("New", EventHandler::new(move |()| {
                    if several {
                        open.set(Open::Accounts);
                    } else if let Some(account) = first {
                        open.set(Open::Naming { account, parent: None, text: String::new() });
                    }
                }))
            }],
        }
        if *open.read() == Open::Accounts {
            Floating {
                anchor: new_button(),
                title: "New folder on".to_owned(),
                items: pick_items,
                on_pick: move |key: String| {
                    if let Ok(uuid) = key.parse() {
                        let account = AccountId::from_uuid(uuid);
                        open.set(Open::Naming { account, parent: None, text: String::new() });
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
        Said { wires, account: None, path: None }
        List::<FolderKey> {
            label: "Folders",
            items,
            style: ListStyle::SourceList,
        }
    }
}
