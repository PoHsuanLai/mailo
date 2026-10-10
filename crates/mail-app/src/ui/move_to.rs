//! "Move to…": a conversation filed into one of its account's own folders.
//!
//! The row's menu and the reader's tools open the one menu on the account's folders; a
//! folder row in the sidebar takes a dropped row. All three file through [`Op::File`], the op a
//! rule's "move to folder" performs, applied by `motion::act`, so the undo and the toast are the
//! ones every other op has.
//!
//! `Op::File` names a label: the server's label that *is* the folder, where mailboxes are labels,
//! and elsewhere the label a move into it files under, named by its path — the same one a rule
//! finds or makes (`mail_store::rules`), so a folder is one label whichever way mail got there.

use super::menu::{MenuItem, Right, Tile, anchor_for, narrowed, palette_groups};
use super::motion::drag::Drag;
use super::motion::{act_all, motion};
use super::picks::with_selection;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::{SqliteStore, Store};
use mail_domain::Label;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;

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
    let labels = store.labels(account.clone()).map_err(|e| e.to_string())?;
    if let Some(label) = labels
        .iter()
        .find(|l| l.origin == LabelOrigin::Provider && l.name == path)
    {
        return Ok(label.id);
    }
    let label = Label {
        id: LabelId::generate(),
        account: account.clone(),
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

/// File each of `threads` on the folder's account into it, with the undo and the toast, as one
/// gesture: one undo puts them all back. Nothing happens to one on another account: a folder is
/// one server's. Returns how many were filed.
pub(in crate::ui) fn file_all_into(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    threads: &[ThreadId],
    folder: &MailboxRef,
) -> usize {
    let ours: Vec<ThreadId> = threads
        .iter()
        .copied()
        .filter(|thread| account_of(store, *thread) == Some(folder.account.clone()))
        .collect();
    if ours.is_empty() {
        return 0;
    }
    match folder_label(store, folder.account.clone(), &folder.path) {
        Ok(label) => {
            let ops = ours
                .into_iter()
                .map(|thread| (thread, Op::File(label)))
                .collect();
            act_all(store, shell, revision, ops)
        }
        Err(why) => {
            eprintln!("move to {}: {why}", folder.path);
            0
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
    file_all_into(
        &store,
        shell,
        revision,
        &with_selection(shell, thread),
        folder,
    );
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

/// The folders `thread` can be moved to, and the move: quire's `PickList` under the control
/// that opened it, its field narrowing the account's folders as it is typed into.
#[component]
pub(in crate::ui) fn MoveMenu(
    thread: ThreadId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    anchor: Option<MountedRef>,
    /// The opener's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
    on_close: EventHandler<()>,
) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let account = account_of(&store, thread);
    let found = account
        .clone()
        .map_or_else(Vec::new, |a| destinations(&store, a));
    let mut query = use_signal(String::new);
    let shown = narrowed(&items(&found), &query());
    rsx! {
        PickList::<String> {
            anchor: anchor_for(anchor, placed),
            label: "Move to",
            placeholder: "Move to…",
            query: query(),
            groups: palette_groups(&shown, AvatarSize::Size22, None),
            empty: if found.is_empty() {
                "No folders".to_owned()
            } else {
                "No folder matches.".to_owned()
            },
            oninput: move |text: String| query.set(text),
            onpick: move |path: String| {
                let Some(account) = account.clone() else { return };
                let store = consume_context::<Arc<SqliteStore>>();
                let folder = MailboxRef { account, path };
                on_close.call(());
                // The whole selection when this row is picked, as one gesture.
                file_all_into(
                    &store,
                    shell,
                    revision,
                    &with_selection(shell, thread),
                    &folder,
                );
            },
            onclose: move |()| on_close.call(()),
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
    let mut open = use_signal(|| Shown::Hidden);
    let mut button = use_signal(|| None::<MountedRef>);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::FolderInput)),
            label: "Move to a folder",
            title: Some("Move to Folder".to_owned()),
            shown: open(),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    button.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onclick: move |_| open.set(if open() == Shown::Visible { Shown::Hidden } else { Shown::Visible }),
        }
        if open() == Shown::Visible {
            MoveMenu {
                thread,
                shell,
                revision,
                anchor: button(),
                on_close: move |_| open.set(Shown::Hidden),
            }
        }
    }
}

#[cfg(test)]
mod tests;
