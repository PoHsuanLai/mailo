//! Under a message's head: what its OpenPGP or S/MIME says, and the passphrase field when its
//! OpenPGP key is locked.

use dioxus::prelude::*;
use mail_domain::{BlobId, MessageId};
use mail_store::SqliteStore;
use std::sync::Arc;

use super::{Busy, Look, Unlock, cached, lookup, seams, short, unlock};
use crate::password::Password;

/// One message's protection, once it is known.
///
/// Draws nothing until then: finding out reads the stored blob and may decrypt, so it runs on a
/// blocking thread and lands here, or is already kept from the last time the message was open.
/// Keyed by the reader on the message and its body, so one message's words are never drawn under
/// another. `landed` is the reader's: moved when a body to show in place of the stored one is
/// found, so the reader draws it.
#[component]
pub(in crate::ui) fn Seal(
    message: MessageId,
    body: Option<BlobId>,
    landed: Signal<u64>,
) -> Element {
    let mut known = use_signal(move || cached(message, body));
    let working = use_signal(|| Busy::Idle);
    let _look = use_resource(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        let secrets = seams().secrets;
        async move {
            if known.peek().is_some() {
                return;
            }
            let found = tokio::task::spawn_blocking(move || {
                lookup(&store, secrets.as_ref(), message, body)
            })
            .await
            .unwrap_or_else(|error| Look::Failed(format!("Opening it stopped: {error}")));
            let shows = matches!(&found, Look::Opened(opened) if opened.shown.is_some());
            known.set(Some(found));
            if shows {
                let mut landed = landed;
                landed += 1;
            }
        }
    });
    // Made here, not in the field: a task spawned from a component that is gone is dropped with
    // it, and a callback runs in the scope that made it.
    let on_unlock = use_callback(move |passphrase: Password| {
        open_with(message, body, passphrase, known, working, landed)
    });
    match known() {
        None | Some(Look::Plain) => rsx! {},
        Some(Look::Failed(why)) => rsx! {
            div { class: "seal", role: "status", aria_label: "Signature and encryption",
                p { class: "seal-line bad", "{why}" }
            }
        },
        Some(Look::Opened(opened)) => rsx! {
            div { class: "seal", role: "status", aria_label: opened.scheme.name(),
                for (at, said) in opened.said.into_iter().enumerate() {
                    p { key: "{at}", class: said.tone.class(), "{said.text}" }
                }
            }
        },
        Some(Look::Locked { key, tried }) => {
            let prompt = format!(
                "This message is encrypted to a key with a passphrase: your key {}.",
                short(key)
            );
            rsx! {
                div { class: "seal", role: "status", aria_label: "OpenPGP",
                    Unlock {
                        prompt,
                        tried,
                        act: "Unlock".to_owned(),
                        working: working(),
                        on_unlock: move |passphrase| on_unlock.call(passphrase),
                    }
                }
            }
        }
    }
}

/// Open the message again with `passphrase`, on a blocking thread, and say what that found.
fn open_with(
    message: MessageId,
    body: Option<BlobId>,
    passphrase: Password,
    mut known: Signal<Option<Look>>,
    mut working: Signal<Busy>,
    mut landed: Signal<u64>,
) {
    if *working.peek() == Busy::Working {
        return;
    }
    working.set(Busy::Working);
    let store = consume_context::<Arc<SqliteStore>>();
    let secrets = seams().secrets;
    // Spawned from a press, which is where a task is polled (F140).
    spawn(async move {
        let found = tokio::task::spawn_blocking(move || {
            unlock(&store, secrets.as_ref(), message, body, passphrase)
        })
        .await
        .unwrap_or_else(|error| Look::Failed(format!("Unlocking stopped: {error}")));
        let shows = matches!(&found, Look::Opened(opened) if opened.shown.is_some());
        known.set(Some(found));
        working.set(Busy::Idle);
        if shows {
            landed += 1;
        }
    });
}
