//! One folder's row, the menu under it, and what its picks do.

use super::super::folder_open;
use super::super::menu::Floating;
use super::super::move_to;
use super::folder_act::{act, messages_word, refused, renamed_path};
use super::folder_parts::{Naming, RenameField, Said, actions, item};
use super::folder_tree::{Kind, Node};
use super::folders::{Note, Open, Spot, Wires, focus_name};
use crate::folder::Refusal;
use crate::view::{Source, folder_of};
use dioxus::prelude::*;
use ds::{
    DataAttr, DataName, Disclosure, DropState, Here, Icon, IconButton, IconButtonVariant, MenuKind,
    MountedRef, PlaceId, Press, Propagation, Switch, TreeItem, TreeShape,
};
use mail_domain::{
    AccountId, Filter, FolderError, FolderWork, Holds, MailboxRef, NonEmpty, Subscription,
};
use mail_store::SqliteStore;
use std::sync::Arc;

/// One folder and, beneath it, what it holds.
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
        account,
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
        account,
        path: node.path.clone(),
    };
    let fetched = label.is_none().then(|| mailbox.clone());
    let data_path = node.path.clone();
    let place = shell.read().places.iter().position(|p| match label {
        Some(id) => p.source == Source::Mail(Filter::HasLabel(id)),
        None => folder_of(p) == Some(&mailbox),
    });
    let count = place.and_then(|index| badges().get(index).copied().flatten());
    let current = place.is_some_and(|index| shell.read().selected == index);
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
    let mut over = use_signal(|| false);
    // Outlined while a dragged row could land here, lit under the pointer: quire's drop rules,
    // the same as a place's.
    let drop = match (takes && move_to::dragging(), over()) {
        (true, true) => DropState::Target,
        (true, false) => DropState::Accepts,
        (false, _) => DropState::Idle,
    };
    let target = mailbox.clone();
    let on_up = move |event: Event<PointerData>| {
        over.set(false);
        if takes && move_to::drop_on(shell, revision, &target) {
            event.stop_propagation();
        }
    };
    let on_enter = move |_: Event<PointerData>| over.set(takes && move_to::dragging());
    let on_leave = move |_: Event<PointerData>| over.set(false);
    let parent = !node.children.is_empty();
    // Open until the person closes it; a folder whose new-folder field or note is showing stays
    // open so they show.
    let mut disclosure = use_signal(|| Disclosure::Open);
    let now = open.read().clone();
    let renaming = match &now {
        Open::Renaming { spot: at, text } if *at == spot => Some(text.clone()),
        _ => None,
    };
    let menu_open =
        matches!(&now, Open::Actions(at) | Open::Confirm { spot: at, .. } if *at == spot);
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
    let menu_spot = spot.clone();
    let closing = spot.clone();
    let rename_spot = spot.clone();
    // The ⋯ button: the menu and its confirmation float beside it, over the sidebar's edge.
    let mut more = use_signal(|| None::<MountedRef>);
    let path = node.path.clone();
    let trailing = rsx! {
        IconButton {
            variant: IconButtonVariant::Strip,
            icon: Icon::Ellipsis,
            label: format!("Actions for {name}"),
            expanded: if menu_open { Switch::On } else { Switch::Off },
            data: folder_data(&data_path),
            mounted: move |event: MountedEvent| more.set(Some(MountedRef(event.data()))),
            propagation: Propagation::Stop,
            onclick: move |_| {
                note.set(None);
                let showing = matches!(&*open.peek(), Open::Actions(at) if *at == menu_spot);
                open.set(if showing { Open::Closed } else { Open::Actions(menu_spot.clone()) });
            },
        }
    };
    // Choosing the folder is its name's press; a folder that is no place has a name only.
    let onselect = place.map(|index| {
        EventHandler::new(move |_: Press| {
            shell.write().select(index);
            pages.set(1);
            if let Some(mailbox) = fetched.clone() {
                folder_open::opened(mailbox, revision);
            }
        })
    });
    // A rename is written where the name is (quire's `editing` slot); taking the slot away,
    // when `open` moves on, ends it.
    let editing = renaming.map(|text| {
        rsx! {
            RenameField {
                value: text,
                on_input: move |text: String| {
                    open.set(Open::Renaming { spot: rename_spot.clone(), text });
                },
                on_commit: move |_| {
                    let Open::Renaming { spot, text } = open.peek().clone() else { return };
                    let to = match renamed_path(&spot.path, &text, delimiter) {
                        Ok(to) if to == spot.path => {
                            open.set(Open::Closed);
                            return;
                        }
                        Ok(to) => to,
                        Err(text) => {
                            note.set(Some(Note { account, path: Some(spot.path.clone()), text }));
                            return;
                        }
                    };
                    let work = FolderWork::Rename { from: spot.path.clone(), to };
                    run(wires, account, Some(spot.path.clone()), work, delimiter);
                },
                on_cancel: move |_| open.set(Open::Closed),
            }
        }
    });
    let below = rsx! {
        Naming {
            wires,
            at: Some(spot.clone()),
            delimiter_of: Callback::new(move |_| delimiter),
        }
        Said { wires, account: Some(account), path: Some(node.path.clone()) }
    };
    // The ⋯'s menus float beside it, so where they sit in the tree does not matter, except that
    // a closed folder hides what it holds: they go after the whole item.
    let menus = rsx! {
        match now {
            Open::Actions(at) if at == spot => rsx! {
                Floating {
                    kind: MenuKind::Slim,
                    anchor: more(),
                    title: name.clone(),
                    items,
                    on_pick: move |key: String| pick(wires, &key, account, path.clone(), delimiter),
                    // A pick that opened a field or the confirmation keeps it open.
                    on_close: move |_| close(open, |now| matches!(now, Open::Actions(at) if *at == closing)),
                }
            },
            Open::Confirm { spot: at, messages } if at == spot => rsx! {
                Floating {
                    kind: MenuKind::Slim,
                    anchor: more(),
                    title: format!("Holds {}. Delete them with the folder?", messages_word(messages)),
                    items: vec![
                        item("delete", Icon::Trash, "Delete folder and mail", Some("This cannot be undone")),
                        item("keep", Icon::X, "Keep the folder", None),
                    ],
                    on_pick: move |key: String| {
                        if key == "delete" {
                            let work = FolderWork::Delete { path: at.path.clone(), non_empty: NonEmpty::Allow };
                            run(wires, account, Some(at.path.clone()), work, delimiter);
                        } else {
                            open.set(Open::Closed);
                        }
                    },
                    on_close: move |_| close(open, |now| matches!(now, Open::Confirm { .. })),
                }
            },
            _ => rsx! {},
        }
    };
    let children = node.children.clone();
    let here = if current {
        Here::Current
    } else {
        Here::Elsewhere
    };
    let count = count.map(|count| u32::try_from(count).unwrap_or(u32::MAX));
    // `.fold` holds the row and what shows under it; `dim` quietens a folder not followed.
    let class = if dim { "fold dim" } else { "fold" };
    if parent {
        let shown = if showing_below {
            Disclosure::Open
        } else {
            disclosure()
        };
        rsx! {
            div { class,
                TreeItem {
                    label: name.clone(),
                    open: shown,
                    on_toggle: move |to| disclosure.set(to),
                    count,
                    here,
                    onselect,
                    trailing,
                    editing: editing.clone(),
                    drop,
                    place: PlaceId(data_path.clone()),
                    onpointerenter: on_enter,
                    onpointerleave: on_leave,
                    onpointerup: on_up,
                    // What the folder has open (its name field, a refusal) first, one step in,
                    // where a new folder inside it goes; then what it holds.
                    {below}
                    for child in children {
                        FolderRow { key: "{child.path}", node: child, account, delimiter, wires }
                    }
                }
                {menus}
            }
        }
    } else {
        rsx! {
            div { class,
                TreeItem {
                    label: name.clone(),
                    open: Disclosure::Closed,
                    on_toggle: |_| {},
                    shape: TreeShape::Leaf,
                    count,
                    here,
                    onselect,
                    trailing,
                    editing: editing.clone(),
                    drop,
                    place: PlaceId(data_path.clone()),
                    onpointerenter: on_enter,
                    onpointerleave: on_leave,
                    onpointerup: on_up,
                }
                {below}
                {menus}
            }
        }
    }
}

/// `data-folder="{path}"` on a folder's ⋯, where a test or a drag finds the folder it names.
fn folder_data(path: &str) -> Vec<DataAttr> {
    DataName::parse(FOLDER_DATA)
        .map(|name| vec![DataAttr::new(name, path)])
        .unwrap_or_default()
}

/// The `data-*` name a folder's path is written under.
const FOLDER_DATA: &str = "folder";

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
        account,
        path: path.clone(),
    };
    match key {
        "new" => {
            open.set(Open::Naming {
                account,
                parent: Some(path),
                text: String::new(),
            });
            focus_name();
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
                account,
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
        account,
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
