//! Read receipts in the reader: a message that asks is answered by the person's setting
//! (`reading.read_receipts`), never by a question on the screen.
//!
//! Whether a message asks is a header the domain message does not carry, so finding out reads
//! the stored raw message. That is done off the thread that draws, for the thread the reader has
//! open and nowhere else — it is reached from [`Receipts`] only, and a test counts the reads of
//! the app's [`Looks`] — and the answer is kept per message and body there, like the list lookup
//! beside it. A message whose body has not been fetched is [`ReceiptState::Unknown`] and is not
//! answered: fetching it to find out would be a POP3 `RETR`, which marks it read.
//!
//! What asking and answering mean is [`mail_core::receipt`]'s, the module `mailo receipt` uses,
//! so the window and the command cannot disagree about it. Under [`ReadReceipts::Never`], the
//! default, nothing is looked up and nothing is answered: the request stays as it came, and a
//! later `mailo receipt` can still answer it. Under [`ReadReceipts::Always`] a receipt is queued
//! the first time a message that asks is opened, and only where [`automatic`] says it is safe.

use crate::settings::{MailSettings, ReadReceipts};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::message::Looks;
use mail_core::receipt::ReceiptState;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_mime::ReturnPath;
use std::sync::{Arc, Mutex};

// One message's standing, with the name of its sender.
pub(in crate::ui) use mail_core::message::Receipt as Standing;

// What a thread's standings are a function of: the messages and the bytes each one has. A body
// that arrives is a new key.
pub(in crate::ui) use mail_core::message::Bodies;

/// What `policy` answers for a message in `mailbox` whose request stands as `state`: a receipt,
/// or nothing at all.
///
/// Pure, so the whole table is a test. Only [`ReadReceipts::Always`] sends, and only for a
/// request nobody has answered, from a message outside Junk, whose receipt goes back to the
/// domain the message came from ([`ReturnPath::Agrees`]). A request whose return path differs,
/// or cannot be told, is left alone: anyone can write `Disposition-Notification-To`, and a
/// receipt to a stranger's address tells the stranger this mailbox reads its mail.
pub(in crate::ui) fn automatic(
    policy: ReadReceipts,
    state: &ReceiptState,
    mailbox: MailboxRole,
) -> Option<ReceiptAnswer> {
    let ReceiptState::Pending(ask) = state else {
        return None;
    };
    let safe = mailbox != MailboxRole::Spam && matches!(ask.return_path, ReturnPath::Agrees);
    (policy == ReadReceipts::Always && safe).then_some(ReceiptAnswer::Sent)
}

/// Held while a receipt is being given, so two windows opening the same message cannot both
/// find it unanswered and both queue one. The store's own answer, read again under it, keeps a
/// message from being answered twice across launches.
static ANSWERING: Mutex<()> = Mutex::new(());

/// Answer, under `policy`, every message among `standings` that asks: look each one up again in
/// the store for where it is now, and queue a receipt through [`mail_core::receipt::answer`] where
/// [`automatic`] says to. Returns the messages a receipt was queued for. Blocking: the reader
/// calls it on a blocking thread.
pub(in crate::ui) fn answer_asked(
    looks: &Looks,
    store: &SqliteStore,
    standings: &[Standing],
    policy: ReadReceipts,
    now: DateTime<Utc>,
) -> Vec<MessageId> {
    let mut sent = Vec::new();
    for standing in standings {
        let Ok(message) = store.message(standing.message) else {
            continue;
        };
        let Some(given) = automatic(policy, &standing.state, message.mailbox) else {
            continue;
        };
        let _held = ANSWERING.lock().unwrap_or_else(|held| held.into_inner());
        // `answer` reads the request's state again and refuses one already answered, here or
        // by an earlier launch, so the lock and the store between them send it once.
        match mail_core::receipt::answer(store, standing.message, given, now) {
            Ok(_) => {
                looks.receipt_again(store, standing.message);
                sent.push(standing.message);
            }
            Err(why) => eprintln!("read receipt: {why}"),
        }
    }
    sent
}

/// The standings already found for every message in `key`, else read for each message and remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(looks: &Looks, store: &SqliteStore, key: &Bodies) -> Vec<Standing> {
    looks.receipts(store, key)
}

/// The open thread's requests, answered by the setting. Draws nothing.
///
/// Keyed by the reader on the thread and its bodies, so a body arriving looks again and one
/// thread's answer is never given for another. The setting is read as the thread opens: changing
/// it later answers the next thread opened, not this one again. A reader with no window around
/// it (a test's) has no settings, and answers nothing.
#[component]
pub(in crate::ui) fn Receipts(bodies: Bodies) -> Element {
    let looks = crate::ui::reading::use_looks();
    let settings = use_hook(try_consume_context::<Signal<MailSettings>>);
    let _answer = use_resource(move || {
        let bodies = bodies.clone();
        let looks = looks.clone();
        let store = consume_context::<Arc<SqliteStore>>();
        let policy = settings
            .map(|settings| settings.peek().reading.read_receipts)
            .unwrap_or_default();
        async move {
            if policy == ReadReceipts::Never {
                return;
            }
            let _ = tokio::task::spawn_blocking(move || {
                let standings = lookup(&looks, &store, &bodies);
                answer_asked(&looks, &store, &standings, policy, Utc::now())
            })
            .await;
        }
    });
    rsx! {}
}

#[cfg(test)]
mod tests;
