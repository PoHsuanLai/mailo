//! What a message has attached, and saving it.
//!
//! Save writes a part that is already here; Download fetches one still on the server and then
//! writes it. The name is the one the file will be written under. There is no file chooser: it
//! lands in the downloads directory, and the notice says where.
//!
//! A message OpenPGP or S/MIME opened to a body of its own lists what is attached inside it —
//! what is stored is only its wrapping, the signature or the ciphertext. Those parts are in
//! memory, in what the reader was opened to; Save writes one off the thread that draws.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_domain::{BlobId, MessageId};
use mail_store::SqliteStore;

use super::super::text::{AttachmentRow, Kept};
use ds::{Glyph, Icon};

/// The rows for one message. `saved` says where the last one went; `downloading` is the part
/// being fetched, whose button stays disabled until the fetch ends.
#[component]
pub(super) fn Attachments(
    message: MessageId,
    body: Option<BlobId>,
    rows: Vec<AttachmentRow>,
    saved: Signal<Option<String>>,
    downloading: Signal<Option<(MessageId, usize)>>,
) -> Element {
    rsx! {
        ul { class: "attachments",
            for row in rows {
                li { key: "{row.index}",
                    Glyph { icon: Icon::Paperclip, size: ds::IconSize::Compact }
                    span { class: "name", "{row.name}" }
                    span { class: "size mono", "{row.size}" }
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: if downloading() == Some((message, row.index)) {
                            match row.kept {
                                Kept::OnServer => "Downloading…",
                                Kept::Here | Kept::Opened => "Saving…",
                            }
                        } else {
                            match row.kept {
                                Kept::Here | Kept::Opened => "Save",
                                Kept::OnServer => "Download",
                            }
                        },
                        availability: super::super::press::available(downloading() != Some((message, row.index))),
                        onclick: {
                            let index = row.index;
                            let name = row.name.clone();
                            let kept = row.kept;
                            super::super::press::on_primary(move || match kept {
                                Kept::Here => save_here(message, index, saved),
                                Kept::Opened => save_opened(message, body, index, saved, downloading),
                                Kept::OnServer => download(message, index, &name, saved, downloading),
                            })
                        },
                    }
                }
            }
        }
    }
}

/// Write a stored part to the downloads directory.
fn save_here(message: MessageId, index: usize, mut saved: Signal<Option<String>>) {
    let store = consume_context::<Arc<SqliteStore>>();
    let where_to = crate::attach::downloads_dir();
    saved.set(Some(
        match crate::attach::save(&store, message, index, &where_to) {
            Ok(path) => format!("Saved to {}", path.display()),
            Err(why) => why,
        },
    ));
}

/// Write a part of what a protected message was opened to, on a blocking thread.
fn save_opened(
    message: MessageId,
    body: Option<BlobId>,
    index: usize,
    mut saved: Signal<Option<String>>,
    mut downloading: Signal<Option<(MessageId, usize)>>,
) {
    if downloading() == Some((message, index)) {
        return;
    }
    downloading.set(Some((message, index)));
    let dir = crate::attach::downloads_dir();
    // Spawned from a press, which is where a task is polled (F140).
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            super::super::pgp::save_attachment(message, body, index, &dir)
        })
        .await;
        saved.set(Some(match done {
            Ok(Ok(path)) => format!("Saved to {}", path.display()),
            Ok(Err(why)) => why,
            Err(error) => format!("Saving stopped before it finished: {error}"),
        }));
        downloading.set(None);
    });
}

/// Fetch a part still on the server, then write it.
fn download(
    message: MessageId,
    index: usize,
    name: &str,
    mut saved: Signal<Option<String>>,
    mut downloading: Signal<Option<(MessageId, usize)>>,
) {
    if downloading() == Some((message, index)) {
        return;
    }
    saved.set(Some(format!("Downloading {name}…")));
    downloading.set(Some((message, index)));
    // Cloned out of the context into the blocking thread: the fetch outlives this click, and
    // `fetch_part` holds the store for the whole download.
    let store = consume_context::<Arc<SqliteStore>>();
    let dir = crate::attach::downloads_dir();
    spawn(async move {
        // `spawn_blocking`, not this task: `fetch_part` opens sockets and builds its own
        // runtime, and `Runtime::block_on` inside an async context panics.
        let done = tokio::task::spawn_blocking(move || {
            crate::attach::fetch_and_save(&store, message, index, &dir, |section| {
                crate::sync::fetch_part(&store, message, section, chrono::Utc::now())
            })
        })
        .await;
        let sentence = match done {
            Ok(Ok(sentence) | Err(sentence)) => sentence,
            Err(error) => format!("The download stopped before it finished: {error}"),
        };
        saved.set(Some(sentence));
        downloading.set(None);
    });
}
