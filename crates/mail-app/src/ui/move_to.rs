//! "Move to…": a conversation filed into one of its account's own folders.
//!
//! The row's strip and the reader's tools open the one [`Menu`] on the account's folders; a
//! folder row in the sidebar takes a dropped row. All three file through [`Op::File`], the op a
//! rule's "move to folder" performs, applied by `motion::act`, so the undo and the toast are the
//! ones every other op has.
//!
//! `Op::File` names a label: the server's label that *is* the folder, where mailboxes are labels,
//! and elsewhere the label a move into it files under, named by its path — the same one a rule
//! finds or makes (`mail_store::rules`), so a folder is one label whichever way mail got there.

use super::icon::{Glyph, Icon};
use super::menu::{Menu, MenuItem, Right, Tile};
use super::motion::drag::Drag;
use super::motion::{act, motion};
use crate::view::Shell;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// Menus with more folders than this get a filter field.
const FILTER_OVER: usize = 7;

/// One folder a conversation can be moved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Destination {
    /// The folder's path, as the server names it and the menu shows it.
    pub path: String,
}

/// The account's own folders that hold mail, by path: never the inbox or a special-use folder,
/// which are places with ops of their own, and never a label that is only a label.
pub(in crate::ui) fn destinations(store: &SqliteStore, account: AccountId) -> Vec<Destination> {
    let mut folders: Vec<Folder> = store
        .folders(account)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.protected().is_none() && f.holds == Holds::Mail)
        .collect();
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    folders
        .into_iter()
        .map(|f| Destination { path: f.path })
        .collect()
}

/// The label `Op::File` files under for the folder at `path`: the server's own label of that
/// name where there is one, made (as the server's) where there is not.
pub(in crate::ui) fn folder_label(
    store: &SqliteStore,
    account: AccountId,
    path: &str,
) -> Result<LabelId, String> {
    let labels = store.labels(account).map_err(|e| e.to_string())?;
    if let Some(label) = labels
        .iter()
        .find(|l| l.origin == LabelOrigin::Provider && l.name == path)
    {
        return Ok(label.id);
    }
    let label = Label {
        id: LabelId::generate(),
        account,
        name: path.to_owned(),
        color: None,
        origin: LabelOrigin::Provider,
    };
    store
        .apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(label.clone())],
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(label.id)
}

/// The account `thread` belongs to.
pub(in crate::ui) fn account_of(store: &SqliteStore, thread: ThreadId) -> Option<AccountId> {
    let loaded = store.thread(thread).ok()?;
    let first = loaded.messages.first()?;
    store.message(*first).ok().map(|m| m.account)
}

/// File `thread` into the folder at `path` on its own account, with the undo and the toast.
/// Nothing happens for a folder on another account: a folder is one server's.
pub(in crate::ui) fn file_into(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    thread: ThreadId,
    folder: &MailboxRef,
) -> bool {
    if account_of(store, thread) != Some(folder.account) {
        return false;
    }
    match folder_label(store, folder.account, &folder.path) {
        Ok(label) => act(store, shell, revision, thread, Op::File(label)),
        Err(why) => {
            eprintln!("move to {}: {why}", folder.path);
            false
        }
    }
}

/// Whether a row is being dragged. Read while drawing, so a folder row lights up for it.
pub(in crate::ui) fn dragging() -> bool {
    motion().is_some_and(|state| matches!(*state.drag.read(), Drag::Live { .. }))
}

/// A row let go over the folder row for `folder`: file it there, as the menu would. Returns
/// whether a dragged row was there to take; the window's own release then has nothing to drop.
pub(in crate::ui) fn drop_on(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    folder: &MailboxRef,
) -> bool {
    let Some(mut state) = motion() else {
        return false;
    };
    let Drag::Live { thread, .. } = state.drag.peek().clone() else {
        return false;
    };
    state.drag.set(Drag::Idle);
    let store = consume_context::<Arc<SqliteStore>>();
    file_into(&store, shell, revision, thread, folder);
    true
}

/// The menu's rows: one per folder, keyed by its path.
pub(in crate::ui) fn items(destinations: &[Destination]) -> Vec<MenuItem> {
    destinations
        .iter()
        .map(|d| MenuItem {
            key: d.path.clone(),
            tile: Tile::Icon(Icon::FolderInput),
            name: d.path.clone(),
            help: None,
            right: Right::None,
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect()
}

/// The folders `thread` can be moved to, and the move.
#[component]
pub(in crate::ui) fn MoveMenu(
    thread: ThreadId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    on_close: EventHandler<()>,
) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let account = account_of(&store, thread);
    let found = account.map_or_else(Vec::new, |a| destinations(&store, a));
    let empty = found.is_empty();
    let filterable = found.len() > FILTER_OVER;
    rsx! {
        div { class: "row-menu move-menu",
            if empty {
                p { class: "hint", "This account has no folders of its own to move to." }
            }
            Menu {
                title: "Move to".to_owned(),
                items: items(&found),
                filterable,
                on_pick: move |path: String| {
                    let Some(account) = account else { return };
                    let store = consume_context::<Arc<SqliteStore>>();
                    let folder = MailboxRef { account, path };
                    on_close.call(());
                    file_into(&store, shell, revision, thread, &folder);
                },
                on_close: move |_| on_close.call(()),
                on_query: |_| {},
                slim: false,
                active: None,
            }
        }
    }
}

/// Move to…, in the reader head's tools, for the conversation that is open.
#[component]
pub(in crate::ui) fn MoveTool(
    thread: ThreadId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    let mut open = use_signal(|| false);
    let label = "Move to a folder";
    rsx! {
        button {
            class: "tool",
            r#type: "button",
            aria_label: "{label}",
            title: "Move to…",
            aria_expanded: if open() { "true" } else { "false" },
            onclick: move |_| open.toggle(),
            Glyph { icon: Icon::FolderInput, class: None }
        }
        if open() {
            div { class: "move-tool",
                MoveMenu { thread, shell, revision, on_close: move |_| open.set(false) }
            }
        }
    }
}

#[cfg(test)]
mod tests;
