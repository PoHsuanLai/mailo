//! One folder's row, the menu under it, and what its picks do.

use super::super::fetching;
use super::super::menu::{Floating, menu_items};
use super::super::move_to;
use super::folder_act::{act, messages_word, refused, renamed_path};
use super::folder_parts::{NameField, Naming, Said, actions};
use super::folder_tree::{Kind, Node};
use super::folders::{Note, Open, Spot, Wires};
use super::tagged;
use crate::ui::view::{Source, folder_of};
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::base::vocab::RowState;
use ds::components::controls::badge::{Badge, BadgeContent, BadgeTone};
use ds::components::controls::button_model::ButtonRole;
use ds::components::lists::row::confirm::RowConfirm;
use ds::components::lists::row::row::{Outline, RowMounted};
use ds::components::menus::pop_up_button::{PopUpButton, PopUpKind};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::SqliteStore;
use mail_core::folder::Refusal;
use mail_domain::{Filter, FolderError, FolderWork, Holds, MailboxRef, NonEmpty, Subscription};
use porter_core::AccountId;
use std::sync::Arc;

/// One folder and, beneath it, what it holds: quire's `Row`, a branch while it has children or
/// something open under it. Its name chooses it, its ⋯ opens the folder's menu, and a
/// right-click opens the same menu.
#[component]
pub(super) fn FolderRow(
    node: Node,
    account: AccountId,
    delimiter: Option<char>,
    wires: Wires,
) -> Element {
    let (mut shell, mut pages, badges, revision) =
        (wires.shell, wires.pages, wires.badges, wires.revision);
    let (mut open, mut note) = (wires.open, wires.note);
    let spot = Spot {
        account: account.clone(),
        path: node.path.clone(),
    };
    let name = node.name.clone();
    let label = match &node.kind {
        Kind::Listed { label, .. } => *label,
        Kind::Implied => None,
    };
    // Where folders are labels its mail is the label's, and the label is a place: listing it
    // is choosing that place. Elsewhere the folder is a place of its own, and choosing it
    // fetches it too.
    let mailbox = MailboxRef {
        account: account.clone(),
        path: node.path.clone(),
    };
    let fetched = label.is_none().then(|| mailbox.clone());
    let place = shell.read().places.iter().position(|p| match label {
        Some(id) => p.source == Source::Mail(Filter::HasLabel(id)),
        None => folder_of(p) == Some(&mailbox),
    });
    let count = place.and_then(|index| badges().get(index).copied().flatten());
    let current = place.is_some_and(|index| shell.read().place_selected(index));
    let dim = matches!(
        node.kind,
        Kind::Listed {
            subscription: Subscription::Unsubscribed,
            ..
        }
    );
    // A folder that holds mail takes a dragged row: filed into it, as "Move to…" files.
    let takes = matches!(
        node.kind,
        Kind::Listed {
            holds: Holds::Mail,
            ..
        }
    );
    let mut over = use_signal(|| DragOver::No);
    // Outlined while a dragged row could land here, lit under the pointer: quire's drop rules,
    // the same as a place's.
    let drop = match (takes && move_to::dragging(), over()) {
        (true, DragOver::Yes) => DropState::Target,
        (true, DragOver::No) => DropState::Accepts,
        (false, _) => DropState::Idle,
    };
    let target = mailbox.clone();
    let on_up = move |event: Event<PointerData>| {
        over.set(DragOver::No);
        if takes && move_to::drop_on(shell, revision, &target) {
            event.stop_propagation();
        }
    };
    let on_enter = move |_: Event<PointerData>| {
        over.set(if takes && move_to::dragging() {
            DragOver::Yes
        } else {
            DragOver::No
        });
    };
    let on_leave = move |_: Event<PointerData>| over.set(DragOver::No);
    let parent = !node.children.is_empty();
    // Open until the person closes it; a folder whose new-folder field or note is showing stays
    // open so they show.
    let mut disclosure = use_signal(|| Shown::Visible);
    let now = open.read().clone();
    let renaming = match &now {
        Open::Renaming { spot: at, text } if *at == spot => Some(text.clone()),
        _ => None,
    };
    let naming_here = matches!(
        &now,
        Open::Naming { account: at, parent: Some(inside), .. }
            if *at == account && *inside == spot.path
    );
    let said_here =
        wires.note.read().as_ref().is_some_and(|note| {
            note.account == account && note.path.as_deref() == Some(&spot.path)
        });
    let showing_below = naming_here || said_here;
    let items = actions(&node);
    let closing = spot.clone();
    let rename_spot = spot.clone();
    // The row itself: the menu and its confirmation hang from it.
    let row_ref = use_signal(|| None::<MountedRef>);
    let path = node.path.clone();
    // Choosing the folder is the row's press; a folder that is no place has a name only. A
    // right-click is the menu's, whatever the folder is.
    let context_spot = spot.clone();
    let onclick = EventHandler::new(move |press: Press| {
        if press.button == ds::base::press::PointerButton::Secondary {
            note.set(None);
            open.set(Open::Actions(context_spot.clone()));
            return;
        }
        let Some(index) = place else { return };
        // Going to another folder ends a rename in progress.
        if matches!(*open.peek(), Open::Renaming { .. }) {
            open.set(Open::Closed);
        }
        shell.write().select(index);
        pages.set(1);
        if let Some(mailbox) = fetched.clone() {
            fetching::opened(mailbox, revision);
        }
    });
    // A rename is written where the name is; taking `renaming` away, when `open` moves on,
    // ends it.
    let editing = renaming.map(|text| {
        rsx! {
            NameField {
                value: text,
                placeholder: "Folder name".to_owned(),
                on_input: move |text: String| {
                    open.set(Open::Renaming { spot: rename_spot.clone(), text });
                },
                on_commit: { let account = account.clone(); move |()| {
                    let Open::Renaming { spot, text } = open.peek().clone() else { return };
                    let to = match renamed_path(&spot.path, &text, delimiter) {
                        Ok(to) if to == spot.path => {
                            open.set(Open::Closed);
                            return;
                        }
                        Ok(to) => to,
                        Err(text) => {
                            note.set(Some(Note { account: account.clone(), path: Some(spot.path.clone()), text }));
                            return;
                        }
                    };
                    let work = FolderWork::Rename { from: spot.path.clone(), to };
                    run(wires, account.clone(), Some(spot.path.clone()), work, delimiter);
                } },
                on_cancel: move |()| open.set(Open::Closed),
            }
        }
    });
    let below = rsx! {
        Naming {
            wires,
            at: Some(spot.clone()),
            delimiter_of: Callback::new(move |_| delimiter),
        }
        Said { wires, account: Some(account.clone()), path: Some(node.path.clone()) }
    };
    // A right-click opens the folder's menu against the row; the ⋯ in the row is its own
    // `PopUpButton` of the same items.
    let menus = match now.clone() {
        Open::Actions(at) if at == spot => rsx! {
            Floating {
                anchor: row_ref(),
                title: name.clone(),
                items: items.clone(),
                on_pick: { let account = account.clone(); move |key: String| pick(wires, &key, account.clone(), path.clone(), delimiter) },
                // A pick that opened a field or the confirmation keeps it open.
                on_close: move |_| close(open, |now| matches!(now, Open::Actions(at) if *at == closing)),
            }
        },
        _ => rsx! {},
    };
    let selection = if current {
        Selection::Selected
    } else {
        Selection::Unselected
    };
    let outline = if parent || showing_below {
        Outline::Branch(if showing_below {
            Shown::Visible
        } else {
            disclosure()
        })
    } else {
        Outline::Leaf
    };
    // Deleting a folder that holds mail asks in the row's own line.
    let confirm = match &now {
        Open::Confirm { spot: at, messages } if *at == spot => {
            let delete = at.clone();
            Some(RowConfirm {
                question: format!(
                    "Delete \u{201c}{name}\u{201d} and the {} in it?",
                    messages_word(*messages)
                ),
                confirm: "Delete Folder and Mail".to_owned(),
                role: ButtonRole::Destructive,
                on_confirm: EventHandler::new({
                    let account = account.clone();
                    move |()| {
                        let work = FolderWork::Delete {
                            path: delete.path.clone(),
                            non_empty: NonEmpty::Allow,
                        };
                        run(
                            wires,
                            account.clone(),
                            Some(delete.path.clone()),
                            work,
                            delimiter,
                        );
                    }
                }),
                on_cancel: EventHandler::new(move |()| open.set(Open::Closed)),
            })
        }
        _ => None,
    };
    let overflow_path = node.path.clone();
    let overflow_name = name.clone();
    let mark_path = node.path.clone();
    let accessory = rsx! {
        super::marks::FolderMark { shell: wires.shell, account: account.clone(), path: mark_path }
        if let Some(count) = count {
            Badge {
                content: BadgeContent::Number(u32::try_from(count).unwrap_or(u32::MAX)),
                tone: BadgeTone::Quiet,
                size: ControlSize::Small,
            }
        }
        PopUpButton::<String> {
            kind: PopUpKind::Overflow,
            title: Some(format!("Actions for {overflow_name}")),
            size: ControlSize::Small,
            items: menu_items("", &items, false),
            onpick: { let account = account.clone(); move |key: String| {
                note.set(None);
                pick(wires, &key, account.clone(), overflow_path.clone(), delimiter);
            } },
        }
    };
    let children = node.children.clone();
    let common = tagged("place", node.path.clone());
    let common = if dim { super::dimmed(common) } else { common };
    rsx! {
        Row {
            title: name.clone(),
            edit: editing,
            accessory: Accessory::Slot(accessory),
            confirm,
            state: RowState { selection, drop, ..RowState::default() },
            outline,
            on_toggle: move |to| disclosure.set(to),
            onclick,
            onmounted: RowMounted::new(move |event: MountedEvent| { let mut at = row_ref; at.set(Some(MountedRef(event.data()))); }),
            onpointerenter: on_enter,
            onpointerleave: on_leave,
            onpointerup: on_up,
            common,
            // What the folder has open (its name field, a refusal) first, one step in, where a
            // new folder inside it goes; then what it holds.
            {below}
            for child in children {
                FolderRow { key: "{child.path}", node: child, account: account.clone(), delimiter, wires }
            }
        }
        {menus}
    }
}

/// Whether the pointer is over a folder that takes the row being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragOver {
    Yes,
    No,
}

/// A menu closed: the section closes it too, unless a pick has already moved on to something
/// else (a name field, the delete confirmation), which `still` tells apart.
fn close(mut open: Signal<Open>, still: impl Fn(&Open) -> bool) {
    if still(&open.peek()) {
        open.set(Open::Closed);
    }
}

/// What a pick in a folder's ⋯ menu does.
fn pick(wires: Wires, key: &str, account: AccountId, path: String, delimiter: Option<char>) {
    let mut open = wires.open;
    let spot = Spot {
        account: account.clone(),
        path: path.clone(),
    };
    match key {
        "new" => {
            open.set(Open::Naming {
                account,
                parent: Some(path),
                text: String::new(),
            });
        }
        "rename" => {
            let text = super::folder_act::leaf(&path, delimiter).to_owned();
            // The field in the name's place takes the keyboard as it mounts, its text selected.
            open.set(Open::Renaming { spot, text });
        }
        "follow" | "unfollow" => {
            let subscription = if key == "follow" {
                Subscription::Subscribed
            } else {
                Subscription::Unsubscribed
            };
            let work = FolderWork::Subscribe {
                path: path.clone(),
                subscription,
            };
            run(wires, account, Some(path), work, delimiter);
        }
        "delete" => {
            let work = FolderWork::Delete {
                path: path.clone(),
                non_empty: NonEmpty::Refuse,
            };
            let store = consume_context::<Arc<SqliteStore>>();
            match act(
                &store,
                wires.shell,
                wires.revision,
                account.clone(),
                work,
                delimiter,
            ) {
                Ok(()) => open.set(Open::Closed),
                Err(Refusal::Folder(FolderError::NotEmpty { messages, .. })) => {
                    open.set(Open::Confirm { spot, messages });
                }
                Err(refusal) => refuse(wires, account, Some(path), &refusal),
            }
        }
        _ => {}
    }
}

/// Make `work`, closing whatever was open, or say why not under `path`'s row.
pub(super) fn run(
    wires: Wires,
    account: AccountId,
    path: Option<String>,
    work: FolderWork,
    delimiter: Option<char>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    match act(
        &store,
        wires.shell,
        wires.revision,
        account.clone(),
        work,
        delimiter,
    ) {
        Ok(()) => {
            let (mut open, mut note) = (wires.open, wires.note);
            open.set(Open::Closed);
            note.set(None);
        }
        Err(refusal) => refuse(wires, account, path, &refusal),
    }
}

fn refuse(wires: Wires, account: AccountId, path: Option<String>, refusal: &Refusal) {
    let mut note = wires.note;
    note.set(Some(Note {
        account,
        path,
        text: refused(refusal),
    }));
}
