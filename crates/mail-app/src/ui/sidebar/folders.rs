//! The Folders section: an account's own mailboxes, nested, and what can be done to them.
//!
//! Each row's ⋯ opens the one [`Menu`]; naming is the one [`Field`], in place of the name. A
//! parent is a `<details>`, so opening and closing it is the document's own state and needs
//! nothing here to remember it.

use super::super::icon::Icon;
use super::super::menu::{Menu, MenuItem};
use super::folder_parts::{Naming, Said, item};
use super::folder_row::FolderRow;
use super::folder_tree::{Section, Show};
use crate::view::Shell;
use dioxus::prelude::*;
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
pub(super) const FOCUS: &str = "(function focusName(tries) {\
    const input = document.querySelector('.fold-edit input');\
    if (input) { input.focus(); input.select(); }\
    else if (tries > 0) { requestAnimationFrame(() => focusName(tries - 1)); }\
})(20)";

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
    rsx! {
        div { class: "s-h",
            "Folders"
            if section.hidden > 0 || show() == Show::All {
                button {
                    r#type: "button",
                    title: if show() == Show::All { "Hide the folders you do not follow" } else { "Show the folders you do not follow" },
                    onclick: move |_| {
                        let mut show = show;
                        show.set(if show() == Show::All { Show::Followed } else { Show::All });
                    },
                    "{toggle}"
                }
            }
            button {
                r#type: "button",
                aria_label: "New folder",
                title: "New folder",
                onclick: move |_| {
                    if several {
                        open.set(Open::Accounts);
                    } else if let Some(account) = first {
                        open.set(Open::Naming { account, parent: None, text: String::new() });
                        dioxus::document::eval(FOCUS);
                    }
                },
                "+ New"
            }
        }
        if *open.read() == Open::Accounts {
            div { class: "fold-menu",
                Menu {
                    title: "New folder on".to_owned(),
                    items: pick_items,
                    filterable: false,
                    on_pick: move |key: String| {
                        if let Ok(uuid) = key.parse() {
                            let account = AccountId::from_uuid(uuid);
                            open.set(Open::Naming { account, parent: None, text: String::new() });
                            dioxus::document::eval(FOCUS);
                        }
                    },
                    on_close: move |_| open.set(Open::Closed),
                    on_query: |_| {},
                    slim: true,
                    active: None,
                }
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
