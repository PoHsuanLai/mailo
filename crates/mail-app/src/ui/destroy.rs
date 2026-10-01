//! Delete forever and Empty Trash in the window: the sheet that asks, and what opens it.
//!
//! When the window offers either is `crate::destroy`'s: only while Trash or Spam is the place
//! shown. Every way in — a row's strip, the selection bar, the list bar's Empty button, the Ctrl T
//! menu — opens the same sheet, which names how many messages go and says it cannot be undone;
//! only its button deletes. Nothing here is on the undo stack, and the toast carries no Undo.

use super::motion::destroy_all;
use super::press::{SheetClose, on_primary};
use crate::destroy::{Bin, Destroying, Reach, bin_shown, doomed, offered, words};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// How many conversations one page of the Empty count reads.
const PAGE: u32 = 500;

/// Ask about `threads`: those of them with mail in the bin shown. Nothing opens outside a bin, or
/// when none of them has mail there.
pub(in crate::ui) fn ask_chosen(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    threads: &[ThreadId],
) {
    let Some(bin) = bin_shown(&shell.peek()) else {
        return;
    };
    let chosen: Vec<ThreadId> = threads
        .iter()
        .copied()
        .filter(|thread| {
            store
                .thread(*thread)
                .is_ok_and(|loaded| offered(Some(bin), &loaded.summary))
        })
        .collect();
    let asking = counted(store, bin, Reach::Chosen, chosen);
    if asking.messages > 0 {
        shell.write().destroying = Some(asking);
    }
}

/// Ask about everything the bin shown holds, in the accounts in view: Empty Trash.
pub(in crate::ui) fn ask_everything(store: &SqliteStore, mut shell: Signal<Shell>) {
    let Some(bin) = bin_shown(&shell.peek()) else {
        return;
    };
    let mut query = shell.peek().query(PAGE);
    let mut threads = Vec::new();
    while let Ok(page) = store.threads(&query, chrono::Utc::now()) {
        threads.extend(page.items.iter().map(|summary| summary.id));
        match page.next {
            Some(next) if !page.items.is_empty() => query.page.after = Some(next),
            _ => break,
        }
    }
    let asking = counted(store, bin, Reach::Everything, threads);
    // An empty bin has nothing to ask about.
    if asking.messages > 0 {
        shell.write().destroying = Some(asking);
    }
}

/// `threads` with the messages each holds in `bin` counted.
fn counted(store: &SqliteStore, bin: Bin, reach: Reach, threads: Vec<ThreadId>) -> Destroying {
    let messages = threads
        .iter()
        .filter_map(|thread| store.thread(*thread).ok())
        .map(|loaded| {
            let messages: Vec<Message> = loaded
                .messages
                .iter()
                .filter_map(|id| store.message(*id).ok())
                .collect();
            doomed(&messages, bin)
        })
        .sum();
    Destroying {
        bin,
        reach,
        threads,
        messages,
    }
}

/// Close the sheet, deleting nothing, and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().destroying = None;
    crate::ui::host::Host::focus_app();
}

/// The sheet's own button: delete what it named, then close.
fn confirm(shell: Signal<Shell>, revision: Signal<u64>) {
    let Some(asked) = shell.peek().destroying.clone() else {
        return;
    };
    close(shell);
    let store = consume_context::<Arc<SqliteStore>>();
    destroy_all(&store, shell, revision, &asked.threads);
}

/// The confirmation, over the window while it is open.
#[component]
pub(in crate::ui) fn DestroySheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let Some(asked) = shell.read().destroying.clone() else {
        return rsx! {};
    };
    let said = words(&asked);
    rsx! {
        div {
            class: "rules-wrap",
            onclick: move |_| close(shell),
            div {
                class: "rules destroy-sheet",
                role: "alertdialog",
                aria_label: "{said.title}",
                onclick: move |event| event.stop_propagation(),
                div { class: "rules-head",
                    h3 { "{said.title}" }
                    SheetClose { label: "Cancel", on_close: move |()| close(shell) }
                }
                div { class: "rules-part",
                    p { class: "destroy-body", "{said.body}" }
                    div { class: "rules-acts",
                        Button {
                            size: ControlSize::Small,
                            label: said.confirm.clone(),
                            icon: Icon::Trash,
                            common: Common {
                                aria_label: Some(said.confirm.clone()),
                                ..Common::default()
                            },
                            onclick: on_primary(move || confirm(shell, revision)),
                        }
                    }
                }
            }
        }
    }
}

/// The list bar's Empty Trash or Empty Spam, while that place is shown.
#[component]
pub(in crate::ui) fn EmptyButton(shell: Signal<Shell>) -> Element {
    let Some(bin) = bin_shown(&shell.read()) else {
        return rsx! {};
    };
    let name = format!("Empty {}", bin.name());
    rsx! {
        Button {
            size: ControlSize::Small,
            label: name.clone(),
            icon: Icon::Trash,
            title: Some(format!("Delete everything in {} forever", bin.name())),
            common: Common {
                aria_label: Some(name.clone()),
                ..Common::default()
            },
            onclick: on_primary(move || {
                let store = consume_context::<Arc<SqliteStore>>();
                ask_everything(&store, shell);
            }),
        }
    }
}
