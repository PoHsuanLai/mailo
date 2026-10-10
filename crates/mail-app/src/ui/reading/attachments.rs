//! What a message has attached, and saving it.
//!
//! A stored picture or PDF leads its row with a thumbnail that opens the viewer (`thumb.rs`,
//! `viewer.rs`); every other row leads with a paperclip.
//!
//! Save writes a part that is already here; Download fetches one still on the server and then
//! writes it. The name is the one the file will be written under. There is no file chooser: it
//! lands in the downloads directory, and a toast says which folder.
//!
//! Each row's progress is a `mail_core::fetch::Download`: Running turns its button Busy, Saved
//! says so in a toast, Failed says why under the row until it is dismissed or tried again.
//!
//! A message OpenPGP or S/MIME opened to a body of its own lists what is attached inside it —
//! what is stored is only its wrapping, the signature or the ciphertext. Those parts are in
//! memory, in what the reader was opened to; Save writes one off the thread that draws.

use ds::components::lists::list::model::ListStyle;
use ds::prelude::*;
use ds::root::common::Common;
use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_core::fetch::{Download, DownloadEffect, DownloadEvent};
use mail_domain::{BlobId, MessageId, Retry};
use mail_store::{SqliteStore, Store as _};

use super::super::text::{AttachmentRow, Kept};
use super::fetch::{self, Again};
use super::thumb::Thumb;
use crate::ui::view::Shell;

type Downloads = Signal<HashMap<usize, Download>>;

/// The rows for one message. Each row's download is its own state, kept here by index.
#[component]
pub(super) fn Attachments(
    message: MessageId,
    body: Option<BlobId>,
    rows: Vec<AttachmentRow>,
    shell: Signal<Shell>,
) -> Element {
    let downloads: Downloads = use_signal(HashMap::new);
    let toasts = use_hook(try_consume_context::<ds::stack::toast_hub::ToastHub>);
    let store = use_context::<Arc<SqliteStore>>();
    let account = use_hook(|| {
        store
            .message(message)
            .map(|m| fetch::account_address(&store, m.account))
            .unwrap_or_default()
    });
    let items: Vec<ListItem<usize>> = rows
        .iter()
        .map(|row| {
            let index = row.index;
            let state = downloads.read().get(&index).cloned().unwrap_or(Download::Idle);
            let busy = matches!(state, Download::Running { .. });
            let label = match row.kept {
                Kept::Here | Kept::Opened => "Save",
                Kept::OnServer => "Download",
            };
            let name = row.name.clone();
            let kept = row.kept;
            let here = kept == Kept::Here;
            let saved_name = name.clone();
            let again_name = name.clone();
            let button = rsx! {
                Button {
                    label,
                    common: Common { aria_label: Some(format!("{label} {name}")), ..Common::default() },
                    availability: if busy { Availability::Busy } else { Availability::Enabled },
                    onclick: super::super::press::on_primary(move || {
                        start(message, body, index, kept, &saved_name, downloads, toasts);
                    }),
                }
            };
            let failure = match &state {
                Download::Failed { retry, why } => {
                    Some(fetch::download_failure(&name, retry, why, &account))
                }
                _ => None,
            };
            let thumb_name = row.name.clone();
            ListItem::row(
                row.index,
                row.name.clone(),
                rsx! {
                    // A stored part leads with its preview (a picture's, a PDF's first page, or the
                    // paperclip) beside the row, on one line; one still on the server, with the
                    // row's own paperclip.
                    if here {
                        div { class: "att-line",
                            Thumb { message, index, name: thumb_name, shell }
                            Row {
                                title: row.name.clone(),
                                detail: Some(TextLine::from(row.size.clone())),
                                accessory: Accessory::Slot(button),
                            }
                        }
                    } else {
                        Row {
                            leading: RowLeading::Icon(Icon::Paperclip),
                            title: row.name.clone(),
                            detail: Some(TextLine::from(row.size.clone())),
                            accessory: Accessory::Slot(button),
                        }
                    }
                    if let Some(failure) = failure {
                        InlineBanner {
                            severity: Severity::Danger,
                            text: failure.text,
                            detail: Some(failure.detail).filter(|d| !d.is_empty()).map(TextLine::from),
                            actions: match failure.again {
                                Again::Offer => Some(rsx! {
                                    Button {
                                        label: "Try Again",
                                        common: Common { aria_label: Some(format!("Try again to download {name}")), ..Common::default() },
                                        onclick: super::super::press::on_primary(move || {
                                            start(message, body, index, kept, &again_name, downloads, toasts);
                                        }),
                                    }
                                }),
                                Again::Withhold => None,
                            },
                            onclose: move |()| {
                                send(downloads, index, DownloadEvent::Dismiss);
                            },
                        }
                    }
                },
            )
        })
        .collect();
    rsx! {
        div { class: "attachments",
            List::<usize> { label: "Attachments", items, style: ListStyle::Inset }
        }
    }
}

/// Move one row's download by `event`; what the machine asks for is returned.
fn send(mut downloads: Downloads, index: usize, event: DownloadEvent) -> Option<DownloadEffect> {
    let from = downloads
        .peek()
        .get(&index)
        .cloned()
        .unwrap_or(Download::Idle);
    let (to, effect) = from.step(event);
    downloads.write().insert(index, to);
    effect
}

type Toasts = Option<ds::stack::toast_hub::ToastHub>;

/// Begin saving one row: from a press, which is where a task is polled (F140).
fn start(
    message: MessageId,
    body: Option<BlobId>,
    index: usize,
    kept: Kept,
    name: &str,
    downloads: Downloads,
    toasts: Toasts,
) {
    if send(downloads, index, DownloadEvent::Start) != Some(DownloadEffect::Begin) {
        return;
    }
    // What is on the server takes a while, and shows in Downloads while it does.
    let saving = match kept {
        Kept::OnServer => crate::ui::downloads::Saving::begin(name),
        Kept::Here | Kept::Opened => crate::ui::downloads::Saving::quick(),
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let fetchers = fetch::fetchers();
    let dir = crate::ui::files::save_dir();
    spawn(async move {
        // `spawn_blocking`, not this task: a fetch waits on the application's runtime
        // (`edge::block_on`), and `Runtime::block_on` inside an async context panics.
        let done = tokio::task::spawn_blocking(move || {
            let saved = match kept {
                Kept::Here => {
                    mail_core::attach::save(&store, message, index, &dir).map_err(String::from)
                }
                Kept::Opened => super::super::pgp::save_attachment(message, body, index, &dir),
                Kept::OnServer => fetch::fetch_then_save(&store, &fetchers, message, index, &dir),
            };
            (saved, crate::ui::downloads::origin(&store, message))
        })
        .await;
        let (done, origin) = match done {
            Ok((saved, origin)) => (Ok(saved), origin),
            Err(error) => (Err(error), None),
        };
        saving.end(
            done.as_ref().ok().and_then(|saved| saved.as_deref().ok()),
            origin,
        );
        let event = match done {
            Ok(Ok(path)) => DownloadEvent::Saved(path),
            Ok(Err(why)) => DownloadEvent::Failed {
                retry: Retry::Now,
                why,
            },
            Err(error) => DownloadEvent::Failed {
                retry: Retry::Now,
                why: format!("it stopped before it finished: {error}"),
            },
        };
        let saved = match &event {
            DownloadEvent::Saved(path) => Some(fetch::saved_toast(path)),
            _ => None,
        };
        send(downloads, index, event);
        if let Some(text) = saved {
            // The notice is the outcome; the row has nothing more to say.
            send(downloads, index, DownloadEvent::Dismiss);
            if let Some(toasts) = toasts {
                toasts.push(text, None);
            }
        }
    });
}
