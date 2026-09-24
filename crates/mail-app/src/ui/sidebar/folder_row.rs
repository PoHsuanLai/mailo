//! One folder's row, the menu under it, and what its picks do.

use super::super::folder_open;
use super::super::menu::Floating;
use super::super::move_to;
use super::folder_act::{act, messages_word, refused, renamed_path};
use super::folder_parts::{NameField, Naming, Said, actions, item};
use super::folder_tree::{Kind, Node};
use super::folders::{FOCUS, Note, Open, Spot, Wires};
use crate::folder::Refusal;
use crate::view::{Source, folder_of};
use dioxus::prelude::*;
use ds::{Icon, MenuKind, MountedRef};
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
    let class = if dim {
        "item fold-row dim"
    } else {
        "item fold-row"
    };
    // A folder that holds mail takes a dragged row: filed into it, as "Move to…" files.
    let takes = matches!(
        node.kind,
        Kind::Listed {
            holds: Holds::Mail,
            ..
        }
    );
    let mut over = use_signal(|| false);
    let class = match (takes && move_to::dragging(), over()) {
        (true, true) => format!("{class} is-drop-target"),
        (true, false) => format!("{class} can-drop"),
        (false, _) => class.to_owned(),
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
    let chev = if parent { "chev" } else { "chev none" };
    let now = open.read().clone();
    let renaming = match &now {
        Open::Renaming { spot: at, text } if *at == spot => Some(text.clone()),
        _ => None,
    };
    let menu_open =
        matches!(&now, Open::Actions(at) | Open::Confirm { spot: at, .. } if *at == spot);
    let items = actions(&node);
    let menu_spot = spot.clone();
    let closing = spot.clone();
    let rename_spot = spot.clone();
    // The ⋯ button: the menu and its confirmation float beside it, over the sidebar's edge.
    let mut more = use_signal(|| None::<MountedRef>);
    let path = node.path.clone();
    let row = rsx! {
        span { class: "{chev}" }
        if let Some(text) = renaming {
            NameField {
                value: text,
                placeholder: "Folder name".to_owned(),
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
        } else if let Some(index) = place {
            button {
                class: "fold-name",
                r#type: "button",
                "data-folder": "{data_path}",
                onclick: move |event| {
                    // Inside a `<summary>`, a click would also open or close the parent.
                    event.prevent_default();
                    event.stop_propagation();
                    shell.write().select(index);
                    pages.set(1);
                    if let Some(mailbox) = fetched.clone() {
                        folder_open::opened(mailbox, revision);
                    }
                },
                "{name}"
            }
        } else {
            span { class: "fold-name", "{name}" }
        }
        if let Some(count) = count {
            span { class: "count", "{count}" }
        }
        button {
            class: "more",
            r#type: "button",
            aria_label: "Actions for {name}",
            aria_expanded: if menu_open { "true" } else { "false" },
            onmounted: move |event: MountedEvent| more.set(Some(MountedRef(event.data()))),
            onclick: move |event| {
                event.prevent_default();
                event.stop_propagation();
                note.set(None);
                let showing = matches!(&*open.peek(), Open::Actions(at) if *at == menu_spot);
                open.set(if showing { Open::Closed } else { Open::Actions(menu_spot.clone()) });
            },
            "⋯"
        }
    };
    let below = rsx! {
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
        Naming {
            wires,
            at: Some(spot.clone()),
            delimiter_of: Callback::new(move |_| delimiter),
        }
        Said { wires, account: Some(account), path: Some(node.path.clone()) }
    };
    let children = node.children.clone();
    if parent {
        rsx! {
            details { class: "fold", open: true,
                summary {
                    class: "{class}",
                    aria_current: if current { "true" } else { "false" },
                    onpointerenter: on_enter,
                    onpointerleave: on_leave,
                    onpointerup: on_up,
                    {row}
                }
                {below}
                div { class: "fold-kids", role: "group",
                    for child in children {
                        FolderRow { key: "{child.path}", node: child, account, delimiter, wires }
                    }
                }
            }
        }
    } else {
        rsx! {
            div { class: "fold",
                div {
                    class: "{class}",
                    aria_current: if current { "true" } else { "false" },
                    onpointerenter: on_enter,
                    onpointerleave: on_leave,
                    onpointerup: on_up,
                    {row}
                }
                {below}
            }
        }
    }
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
            dioxus::document::eval(FOCUS);
        }
        "rename" => {
            let text = super::folder_act::leaf(&path, delimiter).to_owned();
            open.set(Open::Renaming { spot, text });
            dioxus::document::eval(FOCUS);
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
