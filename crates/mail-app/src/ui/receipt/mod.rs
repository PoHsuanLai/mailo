//! Read receipts in the reader: a message that asks is answered by the person's setting
//! (`reading.read_receipts`), or by the person, when the setting is to ask each time.
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
//! later `mailo receipt` can still answer it. Under [`ReadReceipts::Ask`] each request outside
//! Junk is a line over the conversation, Send Receipt or Don't Send, and nothing goes until one
//! is pressed. Under [`ReadReceipts::Always`] a receipt is queued the first time a message that
//! asks is opened, and only where [`automatic`] says it is safe.

use crate::settings::{MailSettings, ReadReceipts};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::message::Looks;
use mail_core::receipt::ReceiptState;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_mime::ReturnPath;
use std::sync::{Arc, Mutex};

use super::copy::{controls, status};
use super::press::on_primary;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Bezel;
use ds::prelude::*;
use ds::root::common::Common;

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

/// Whether, under `policy`, a message in `mailbox` whose request stands as `state` is a question
/// for the person: only under [`ReadReceipts::Ask`], only for a request nobody has answered, and
/// never in Junk, where even the question would tell the person nothing they want to know.
///
/// Pure, so the whole table is a test. A request whose receipt would go to another domain is
/// still asked about, with the address said beside it ([`Question::elsewhere`]): the person
/// decides, knowing where it would go.
pub(in crate::ui) fn asks(
    policy: ReadReceipts,
    state: &ReceiptState,
    mailbox: MailboxRole,
) -> bool {
    policy == ReadReceipts::Ask
        && mailbox != MailboxRole::Spam
        && matches!(state, ReceiptState::Pending(_))
}

/// The id of a question's Send Receipt button, for `message`.
pub(in crate::ui) fn send_id(message: MessageId) -> String {
    format!("receipt-send-{message}")
}

/// The id of a question's Don't Send button, for `message`.
pub(in crate::ui) fn decline_id(message: MessageId) -> String {
    format!("receipt-decline-{message}")
}

/// One request put to the person under [`ReadReceipts::Ask`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Question {
    pub(in crate::ui) message: MessageId,
    /// The sender's name, else their address.
    pub(in crate::ui) sender: String,
    /// Where the receipt would go, when that is not the domain the message came back from.
    pub(in crate::ui) elsewhere: Option<String>,
}

/// The requests among `standings` that `policy` puts to the person, each looked up again in the
/// store for where its message is now. Blocking: the reader calls it on a blocking thread.
pub(in crate::ui) fn questions(
    store: &SqliteStore,
    standings: &[Standing],
    policy: ReadReceipts,
) -> Vec<Question> {
    standings
        .iter()
        .filter_map(|standing| {
            let message = store.message(standing.message).ok()?;
            if !asks(policy, &standing.state, message.mailbox) {
                return None;
            }
            let ReceiptState::Pending(ask) = &standing.state else {
                return None;
            };
            let elsewhere = (!matches!(ask.return_path, ReturnPath::Agrees)).then(|| {
                ask.to
                    .iter()
                    .map(|to| to.email.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            Some(Question {
                message: standing.message,
                sender: standing.sender.clone(),
                elsewhere,
            })
        })
        .collect()
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
            Err(why) => log::warn!("read receipt: {why}"),
        }
    }
    sent
}

/// The person's answer to one [`Question`]: queue the receipt, or decline it, through
/// [`mail_core::receipt::answer`], which refuses a request already answered. Blocking.
pub(in crate::ui) fn answer_one(
    looks: &Looks,
    store: &SqliteStore,
    message: MessageId,
    given: ReceiptAnswer,
    now: DateTime<Utc>,
) {
    let _held = ANSWERING.lock().unwrap_or_else(|held| held.into_inner());
    match mail_core::receipt::answer(store, message, given, now) {
        Ok(_) => looks.receipt_again(store, message),
        Err(why) => log::warn!("read receipt: {why}"),
    }
}

/// The standings already found for every message in `key`, else read for each message and remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(looks: &Looks, store: &SqliteStore, key: &Bodies) -> Vec<Standing> {
    looks.receipts(store, key)
}

/// The open thread's requests, answered by the setting, or put to the person under
/// [`ReadReceipts::Ask`]. Draws nothing but those questions.
///
/// Keyed by the reader on the thread and its bodies, so a body arriving looks again and one
/// thread's answer is never given for another. The setting is read as the thread opens: changing
/// it later answers the next thread opened, not this one again. A reader with no window around
/// it (a test's) has no settings, and answers nothing.
#[component]
pub(in crate::ui) fn Receipts(bodies: Bodies) -> Element {
    let looks = crate::ui::reading::use_looks();
    let settings = use_hook(try_consume_context::<Signal<MailSettings>>);
    let store = use_context::<Arc<SqliteStore>>();
    // The questions answered in this reader, gone from the screen as soon as each answer is kept
    // rather than when the store's answer is read again.
    let answered = use_signal(Vec::<MessageId>::new);
    let asked = {
        let looks = looks.clone();
        let store = store.clone();
        use_resource(move || {
            let bodies = bodies.clone();
            let looks = looks.clone();
            let store = store.clone();
            let policy = settings
                .map(|settings| settings.peek().reading.read_receipts)
                .unwrap_or_default();
            async move {
                let asked = tokio::task::spawn_blocking(move || match policy {
                    ReadReceipts::Never => Vec::new(),
                    ReadReceipts::Ask => {
                        let standings = lookup(&looks, &store, &bodies);
                        questions(&store, &standings, policy)
                    }
                    ReadReceipts::Always => {
                        let standings = lookup(&looks, &store, &bodies);
                        answer_asked(&looks, &store, &standings, policy, Utc::now());
                        Vec::new()
                    }
                })
                .await;
                // A lookup that panicked asks nothing; the reader goes on without the line.
                asked.unwrap_or_default()
            }
        })
    };
    let shown: Vec<Question> = asked
        .read()
        .iter()
        .flatten()
        .filter(|question| !answered.read().contains(&question.message))
        .cloned()
        .collect();
    rsx! {
        for question in shown {
            Asking { key: "{question.message}", question, answered }
        }
    }
}

/// One question's line: who asked, where the receipt would go when that is somewhere else, and
/// the two answers.
#[component]
fn Asking(question: Question, answered: Signal<Vec<MessageId>>) -> Element {
    let looks = crate::ui::reading::use_looks();
    let store = use_context::<Arc<SqliteStore>>();
    let message = question.message;
    let give = move |given: ReceiptAnswer| {
        let looks = looks.clone();
        let store = store.clone();
        let mut answered = answered;
        move || {
            let looks = looks.clone();
            let store = store.clone();
            // The line goes once the answer is kept, not before: taking it away first would end
            // this component, and the task with it, before the answer was given. A second press
            // meanwhile is refused by `answer`, which finds the request answered.
            spawn(async move {
                let given = tokio::task::spawn_blocking(move || {
                    answer_one(&looks, &store, message, given, Utc::now());
                })
                .await;
                if let Err(why) = given {
                    log::warn!("read receipt: {why}");
                }
                answered.write().push(message);
            });
        }
    };
    let send = give(ReceiptAnswer::Sent);
    let decline = give(ReceiptAnswer::Declined);
    rsx! {
        p { class: "receipt-ask", role: "status",
            Label { text: status::receipt_asked(&question.sender), role: LabelRole::Tertiary, style: LabelStyle::Footnote }
            if let Some(to) = &question.elsewhere {
                Label { text: status::receipt_goes_elsewhere(to), role: LabelRole::Tertiary, style: LabelStyle::Footnote }
            }
            Button {
                bezel: Bezel::Inline,
                label: controls::SEND_RECEIPT,
                common: Common { id: Some(send_id(message)), aria_label: Some(controls::SEND_RECEIPT.to_owned()), ..Common::default() },
                onclick: on_primary(send),
            }
            Button {
                bezel: Bezel::Inline,
                label: controls::DONT_SEND_RECEIPT,
                common: Common { id: Some(decline_id(message)), aria_label: Some(controls::DONT_SEND_RECEIPT.to_owned()), ..Common::default() },
                onclick: on_primary(decline),
            }
        }
    }
}

#[cfg(test)]
mod tests;
