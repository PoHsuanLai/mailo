//! "Move to…": a conversation filed into one of its account's own folders; and "Copy to…": put
//! in one of them as well, and left where it is.
//!
//! The row's menu and the reader's tools open the one menu on the account's folders; a
//! folder row in the sidebar takes a dropped row. All three file through [`Op::File`], the op a
//! rule's "move to folder" performs, applied by `motion::act`, so the undo and the toast are the
//! ones every other op has. A copy is [`Op::Copy`], from the same menus and the same list.
//!
//! Both stay within the account. A folder is one server's, and neither goes to another
//! account's: that would be an upload of the message there, which a move or a copy within one
//! server is not. Copy to… says so above its folders.
//!
//! `Op::File` names a label: the server's label that *is* the folder, where mailboxes are labels,
//! and elsewhere the label a move into it files under, named by its path — the same one a rule
//! finds or makes (`mail_store::rules`), so a folder is one label whichever way mail got there.

use super::menu::{MenuItem, anchor_for, narrowed, palette_groups};
use super::motion::drag::Drag;
use super::motion::{act_all, motion};
use super::picks::with_selection;
use crate::ui::copy;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use mail_client::Client;
use mail_domain::Label;
use mail_domain::*;
use porter_core::AccountId;

/// What a pick in the menu does with the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Gesture {
    /// Out of where it is, into the folder: [`Op::File`].
    Move,
    /// Into the folder as well: [`Op::Copy`].
    Copy,
}

impl Gesture {
    /// The op that puts a conversation in the folder `label` names.
    fn op(self, label: LabelId) -> Op {
        match self {
            Gesture::Move => Op::File(label),
            Gesture::Copy => Op::Copy(label),
        }
    }
}

/// One folder a conversation can be moved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Destination {
    /// The folder's path, as the server names it and the menu shows it.
    pub path: String,
}

/// The account's own folders that hold mail, by path: never the inbox or a special-use folder,
/// which are places with ops of their own, never a label that is only a label, and never a
/// mailbox with no name of the person's ([`Folder::named`]).
pub(in crate::ui) fn destinations(client: &Client, account: AccountId) -> Vec<Destination> {
    let mut folders: Vec<Folder> = client
        .stored()
        .folders(account)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.protected().is_none() && f.named() && f.holds == Holds::Mail)
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
    client: &Client,
    account: AccountId,
    path: &str,
) -> Result<LabelId, String> {
    let labels = client
        .stored()
        .labels(account.clone())
        .map_err(|e| e.to_string())?;
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
    client
        .stored()
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
pub(in crate::ui) fn account_of(client: &Client, thread: ThreadId) -> Option<AccountId> {
    let loaded = client.stored().thread(thread).ok()?;
    let first = loaded.messages.first()?;
    client.stored().message(*first).ok().map(|m| m.account)
}

/// File each of `threads` on the folder's account into it, with the undo and the toast, as one
/// gesture: one undo puts them all back. Nothing happens to one on another account: a folder is
/// one server's. Returns how many were filed.
pub(in crate::ui) fn file_all_into(
    client: &Client,
    shell: Signal<Shell>,
    threads: &[ThreadId],
    folder: &MailboxRef,
) -> usize {
    put_all_into(client, shell, threads, folder, Gesture::Move)
}

/// [`file_all_into`], moving or copying as `gesture` says.
pub(in crate::ui) fn put_all_into(
    client: &Client,
    shell: Signal<Shell>,
    threads: &[ThreadId],
    folder: &MailboxRef,
    gesture: Gesture,
) -> usize {
    let ours: Vec<ThreadId> = threads
        .iter()
        .copied()
        .filter(|thread| account_of(client, *thread) == Some(folder.account.clone()))
        .collect();
    if ours.is_empty() {
        return 0;
    }
    match folder_label(client, folder.account.clone(), &folder.path) {
        Ok(label) => {
            let ops = ours
                .into_iter()
                .map(|thread| (thread, gesture.op(label)))
                .collect();
            act_all(client, shell, ops)
        }
        Err(why) => {
            log::warn!("{gesture:?} to {}: {why}", folder.path);
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
pub(in crate::ui) fn drop_on(shell: Signal<Shell>, folder: &MailboxRef) -> bool {
    let Some(mut state) = motion() else {
        return false;
    };
    let Drag::Live { thread, .. } = state.drag.peek().clone() else {
        return false;
    };
    state.drag.set(Drag::Idle);
    let client = crate::ui::live::client();
    file_all_into(&client, shell, &with_selection(shell, thread), folder);
    true
}

/// The menu's rows: one per folder, keyed by its path.
pub(in crate::ui) fn items(destinations: &[Destination]) -> Vec<MenuItem> {
    destinations
        .iter()
        .map(|d| MenuItem::icon(d.path.clone(), Icon::FolderInput, d.path.clone()))
        .collect()
}

/// The folders `thread` can be moved or copied to, and the move or the copy: quire's `PickList`
/// under the control that opened it, its field narrowing the account's folders as it is typed
/// into.
#[component]
pub(in crate::ui) fn MoveMenu(
    thread: ThreadId,
    shell: Signal<Shell>,
    anchor: Option<MountedRef>,
    /// The opener's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
    /// What a pick does: Move to…'s, unless Copy to… opened it.
    #[props(default = Gesture::Move)]
    gesture: Gesture,
    on_close: EventHandler<()>,
) -> Element {
    let client = crate::ui::live::client();
    let account = account_of(&client, thread);
    let found = account
        .clone()
        .map_or_else(Vec::new, |a| destinations(&client, a));
    let mut query = use_signal(String::new);
    let rows = match (gesture, &account) {
        // Said above the folders: they are this account's, and no other's.
        (Gesture::Copy, Some(account)) => {
            let address = shell
                .read()
                .accounts
                .iter()
                .find(|(_, id)| id == account)
                .map(|(address, _)| address.clone())
                .unwrap_or_default();
            within(items(&found), &copy::menus::copy_within(&address))
        }
        _ => items(&found),
    };
    let shown = narrowed(&rows, &query());
    let (label, placeholder) = match gesture {
        Gesture::Move => (copy::menus::MOVE_TO_LIST, copy::menus::MOVE_TO),
        Gesture::Copy => (copy::menus::COPY_TO_LIST, copy::menus::COPY_TO),
    };
    rsx! {
        PickList::<String> {
            anchor: anchor_for(anchor, placed),
            label,
            placeholder,
            query: query(),
            groups: palette_groups(&shown, AvatarSize::Size22, None),
            empty: if found.is_empty() {
                copy::empty::NO_FOLDERS.to_owned()
            } else {
                copy::empty::NO_FOLDER_MATCHES.to_owned()
            },
            oninput: move |text: String| query.set(text),
            onpick: move |path: String| {
                let Some(account) = account.clone() else { return };
                let client = crate::ui::live::client();
                let folder = MailboxRef { account, path };
                on_close.call(());
                // The whole selection when this row is picked, as one gesture.
                put_all_into(
                    &client,
                    shell,
                    &with_selection(shell, thread),
                    &folder,
                    gesture,
                );
            },
            onclose: move |()| on_close.call(()),
        }
    }
}

/// `rows` as one group, titled `title`.
fn within(rows: Vec<MenuItem>, title: &str) -> Vec<MenuItem> {
    rows.into_iter()
        .map(|row| MenuItem {
            group: Some(title.to_owned()),
            ..row
        })
        .collect()
}

#[cfg(test)]
mod tests;
