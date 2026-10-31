//! Delete Forever and Empty Trash in the window: the alert that asks, and what opens it.
//!
//! Empty Trash is offered while Trash or Spam is the place shown (`crate::ui::bin`); Delete
//! Forever wherever a conversation already in one of them is. Every way in (a row's menu, the
//! selection bar, the list bar's Empty button, ⌘K) opens the same alert, which names how many
//! messages go and says it cannot be undone; only its button deletes. Nothing here is on the undo
//! stack, and the toast carries no Undo.

use super::motion::destroy_all;
use super::press::on_primary;
use crate::ui::bin::{Bin, Destroying, Reach, bin_shown, doomed, offered, words};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_client::Client;
use mail_domain::*;
use mail_ui_model::copy;

/// How many conversations one page of the Empty count reads.
const PAGE: u32 = 500;

/// Ask about `threads`: those of them with mail in the bin shown or, with no bin shown (a
/// search), in the bin the first of them is in. Nothing opens when none of them has mail there.
///
/// The conversations are read off the thread that draws, and the alert opens when they are.
pub(in crate::ui) fn ask_chosen(client: &Client, shell: Signal<Shell>, threads: &[ThreadId]) {
    let shown = bin_shown(&shell.peek());
    let client = client.clone();
    let threads = threads.to_vec();
    opened(shell, move || chosen_in(&client, shown, &threads));
}

/// What [`ask_chosen`] asks, once read.
fn chosen_in(client: &Client, shown: Option<Bin>, threads: &[ThreadId]) -> Option<Destroying> {
    let bin = shown.or_else(|| {
        let first = client.stored().thread(*threads.first()?).ok()?;
        [Bin::Trash, Bin::Spam]
            .into_iter()
            .find(|bin| offered(Some(*bin), &first.summary))
    })?;
    let chosen: Vec<ThreadId> = threads
        .iter()
        .copied()
        .filter(|thread| {
            client
                .stored()
                .thread(*thread)
                .is_ok_and(|loaded| offered(Some(bin), &loaded.summary))
        })
        .collect();
    Some(counted(client, bin, Reach::Chosen, chosen))
}

/// Ask about everything the bin shown holds, in the accounts in view: Empty Trash.
///
/// The bin is paged through off the thread that draws, and the alert opens when it has been.
pub(in crate::ui) fn ask_everything(client: &Client, shell: Signal<Shell>) {
    let Some(bin) = bin_shown(&shell.peek()) else {
        return;
    };
    // The whole bin, whatever the quick filter shows of it.
    let query = shell.peek().whole_query(PAGE);
    let client = client.clone();
    opened(shell, move || everything_in(&client, bin, query));
}

/// What [`ask_everything`] asks, once read.
fn everything_in(client: &Client, bin: Bin, mut query: Query) -> Option<Destroying> {
    let mut threads = Vec::new();
    while let Ok(page) = client.stored().threads(&query, chrono::Utc::now()) {
        threads.extend(page.items.iter().map(|summary| summary.id));
        match page.next {
            Some(next) if !page.items.is_empty() => query.page.after = Some(next),
            _ => break,
        }
    }
    Some(counted(client, bin, Reach::Everything, threads))
}

/// Read what `ask` makes of the store on a blocking thread, then open the alert on it. An empty
/// bin has nothing to ask about, so nothing opens.
fn opened(mut shell: Signal<Shell>, ask: impl FnOnce() -> Option<Destroying> + Send + 'static) {
    spawn(async move {
        match tokio::task::spawn_blocking(ask).await {
            Ok(Some(asking)) if asking.messages > 0 => shell.write().destroying = Some(asking),
            Ok(_) => {}
            Err(why) => log::warn!("deleting: what to ask about was not read: {why}"),
        }
    });
}

/// `threads` with the messages each holds in `bin` counted.
fn counted(client: &Client, bin: Bin, reach: Reach, threads: Vec<ThreadId>) -> Destroying {
    let messages = threads
        .iter()
        .filter_map(|thread| client.stored().thread(*thread).ok())
        .map(|loaded| {
            let messages: Vec<Message> = loaded
                .messages
                .iter()
                .filter_map(|id| client.stored().message(*id).ok())
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

/// Close the alert, deleting nothing, and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().destroying = None;
    crate::ui::host::Host::focus_app();
}

/// The alert's own button: delete what it named, then close.
fn confirm(shell: Signal<Shell>) {
    let Some(asked) = shell.peek().destroying.clone() else {
        return;
    };
    close(shell);
    let client = crate::ui::live::client();
    destroy_all(&client, shell, &asked.threads);
}

/// The confirmation, quire's critical alert over the window while it is open: its Cancel answers
/// Escape and only its destructive button deletes.
#[component]
pub(in crate::ui) fn DestroyAlert(shell: Signal<Shell>) -> Element {
    let Some(asked) = shell.read().destroying.clone() else {
        return rsx! {};
    };
    let said = words(&asked);
    rsx! {
        Alert {
            title: said.title,
            message: Some(TextLine::from(said.body)),
            style: AlertStyle::Critical,
            buttons: vec![
                AlertButton::new(
                    said.confirm.clone(),
                    AlertRole::Destructive,
                    EventHandler::new(move |()| confirm(shell)),
                )
                .with_common(Common {
                    aria_label: Some(said.confirm),
                    ..Common::default()
                }),
                AlertButton::new(
                    copy::controls::CANCEL,
                    AlertRole::Cancel,
                    EventHandler::new(move |()| close(shell)),
                ),
            ],
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
        // A plain toolbar icon beside the others; its name is its tip, and it asks before it acts.
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: name.clone(),
            icon: Icon::Trash,
            title: Some(name.clone()),
            common: Common {
                aria_label: Some(name.clone()),
                ..Common::default()
            },
            onclick: on_primary(move || {
                let client = crate::ui::live::client();
                ask_everything(&client, shell);
            }),
        }
    }
}
