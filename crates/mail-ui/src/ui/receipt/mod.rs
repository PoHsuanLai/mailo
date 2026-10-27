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
//! What asking and answering mean is [`mail_client::receipt`]'s, the module `mailo receipt` uses,
//! so the window and the command cannot disagree about it. Under [`ReadReceipts::Never`], the
//! default, nothing is looked up and nothing is answered: the request stays as it came, and a
//! later `mailo receipt` can still answer it. Under [`ReadReceipts::Ask`] each request outside
//! Junk is a small mark in its message's header (and two rows in the message's menu), Send
//! Receipt or Don't Send, and nothing goes until one is chosen. Under [`ReadReceipts::Always`] a receipt is queued the first time a message that
//! asks is opened, and only where [`automatic`] says it is safe.

use crate::settings::{MailSettings, ReadReceipts};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_client::Client;
use mail_client::message::Looks;
use mail_client::receipt::ReceiptState;
use mail_domain::*;
use mail_mime::{ReceiptAsk, ReturnPath};
use std::sync::Mutex;

use super::copy::{controls, status, tooltips};
use super::press::on_primary;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;

// One message's standing, with the name of its sender.
pub(in crate::ui) use mail_client::message::Receipt as Standing;

// What a thread's standings are a function of: the messages and the bytes each one has. A body
// that arrives is a new key.
pub(in crate::ui) use mail_client::message::Bodies;

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
/// still asked about, with the address said beside it ([`Question::destination`]): the person
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

/// Where a receipt would go, as far as the message's return path says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Destination {
    /// To the domain the message came back from: nothing to say.
    Agreed,
    /// To these addresses, which are not on that domain.
    Elsewhere(String),
    /// To these addresses, with no return path to compare them with.
    Unchecked(String),
}

/// One request put to the person under [`ReadReceipts::Ask`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Question {
    pub(in crate::ui) message: MessageId,
    /// The sender's name, else their address.
    pub(in crate::ui) sender: String,
    pub(in crate::ui) destination: Destination,
}

/// Where a receipt for `ask` would go, as it is to be said.
fn destination(ask: &ReceiptAsk) -> Destination {
    let to = || {
        ask.to
            .iter()
            .map(|to| to.email.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match ask.return_path {
        ReturnPath::Agrees => Destination::Agreed,
        ReturnPath::Differs { .. } => Destination::Elsewhere(to()),
        ReturnPath::Unknown => Destination::Unchecked(to()),
    }
}

/// The requests among `standings` that `policy` puts to the person, each looked up again in the
/// client for where its message is now. Blocking: the reader calls it on a blocking thread.
pub(in crate::ui) fn questions(
    client: &Client,
    standings: &[Standing],
    policy: ReadReceipts,
) -> Vec<Question> {
    standings
        .iter()
        .filter_map(|standing| {
            let ReceiptState::Pending(ask) = &standing.state else {
                return None;
            };
            let message = client
                .stored()
                .message(standing.message)
                .map_err(|why| {
                    log::warn!("a message that asks for a receipt could not be read: {why}")
                })
                .ok()?;
            if !asks(policy, &standing.state, message.mailbox) {
                return None;
            }
            Some(Question {
                message: standing.message,
                sender: standing.sender.clone(),
                destination: destination(ask),
            })
        })
        .collect()
}

/// Held while a receipt is being given, so two windows opening the same message cannot both
/// find it unanswered and both queue one. The client's own answer, read again under it, keeps a
/// message from being answered twice across launches.
static ANSWERING: Mutex<()> = Mutex::new(());

/// Answer, under `policy`, every message among `standings` that asks: look each one up again in
/// the client for where it is now, and queue a receipt through [`mail_client::receipt::answer`] where
/// [`automatic`] says to. Returns the messages a receipt was queued for. Blocking: the reader
/// calls it on a blocking thread.
pub(in crate::ui) fn answer_asked(
    looks: &Looks,
    client: &Client,
    standings: &[Standing],
    policy: ReadReceipts,
    now: DateTime<Utc>,
) -> Vec<MessageId> {
    let mut sent = Vec::new();
    for standing in standings {
        let Ok(message) = client.stored().message(standing.message) else {
            continue;
        };
        let Some(given) = automatic(policy, &standing.state, message.mailbox) else {
            continue;
        };
        let _held = ANSWERING.lock().unwrap_or_else(|held| held.into_inner());
        // `answer` reads the request's state again and refuses one already answered, here or
        // by an earlier launch, so the lock and the client between them send it once.
        match client.receipt().answer(standing.message, given, now) {
            Ok(_) => {
                client.looking(looks).receipt_again(standing.message);
                sent.push(standing.message);
            }
            Err(why) => log::warn!("read receipt: {why}"),
        }
    }
    sent
}

/// The person's answer to one [`Question`]: queue the receipt, or decline it, through
/// [`mail_core::receipt::answer`], which refuses a request already answered. Whether it was kept.
/// Blocking.
pub(in crate::ui) fn answer_one(
    looks: &Looks,
    client: &Client,
    message: MessageId,
    given: ReceiptAnswer,
    now: DateTime<Utc>,
) -> bool {
    let _held = ANSWERING.lock().unwrap_or_else(|held| held.into_inner());
    match client.receipt().answer(message, given, now) {
        Ok(_) => {
            client.looking(looks).receipt_again(message);
            true
        }
        Err(why) => {
            log::warn!("read receipt: {why}");
            false
        }
    }
}

/// The standings already found for every message in `key`, else read for each message and remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(looks: &Looks, client: &Client, key: &Bodies) -> Vec<Standing> {
    client.looking(looks).receipts(key)
}

/// The requests the open thread's messages put to the person under [`ReadReceipts::Ask`], kept
/// for the headers to draw a mark from: the reader provides it, [`Receipts`] fills it, and a
/// message's mark and menu read it ([`ReceiptMark`]). Empty when nothing asks.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Asks(pub Signal<Vec<Question>>);

/// The open thread's requests, answered by the setting, or put to the person under
/// [`ReadReceipts::Ask`] through the reader's [`Asks`]. Draws nothing.
///
/// Keyed by the reader on the thread and its bodies, so a body arriving looks again and one
/// thread's answer is never given for another. The setting is read as the thread opens: changing
/// it later answers the next thread opened, not this one again. A reader with no window around
/// it (a test's) has no settings, and answers nothing.
#[component]
pub(in crate::ui) fn Receipts(bodies: Bodies) -> Element {
    let looks = crate::ui::reading::use_looks();
    let settings = use_hook(try_consume_context::<Signal<MailSettings>>);
    let client = crate::ui::live::client();
    let asks = try_consume_context::<Asks>();
    let asked = {
        let looks = looks.clone();
        let client = client.clone();
        use_resource(move || {
            let bodies = bodies.clone();
            let looks = looks.clone();
            let client = client.clone();
            let policy = settings
                .map(|settings| settings.peek().reading.read_receipts)
                .unwrap_or_default();
            async move {
                let asked = tokio::task::spawn_blocking(move || match policy {
                    ReadReceipts::Never => Vec::new(),
                    ReadReceipts::Ask => {
                        let standings = lookup(&looks, &client, &bodies);
                        questions(&client, &standings, policy)
                    }
                    ReadReceipts::Always => {
                        let standings = lookup(&looks, &client, &bodies);
                        answer_asked(&looks, &client, &standings, policy, Utc::now());
                        Vec::new()
                    }
                })
                .await;
                // A lookup that panicked asks nothing; the reader goes on without the mark.
                asked.unwrap_or_default()
            }
        })
    };
    use_effect(move || {
        if let (Some(Asks(mut asks)), Some(found)) = (asks, asked.read().as_ref()) {
            asks.set(found.clone());
        }
    });
    // A thread left takes its questions with it.
    use_drop(move || {
        if let Some(Asks(mut asks)) = asks {
            asks.set(Vec::new());
        }
    });
    rsx! {}
}

/// The key of Send Receipt in a message's menu and its mark's popover.
pub(in crate::ui) const SEND_KEY: &str = "receipt-send";
/// The key of Don't Send in a message's menu and its mark's popover.
pub(in crate::ui) const DECLINE_KEY: &str = "receipt-decline";

/// The two answers, as menu rows.
pub(in crate::ui) fn items() -> Vec<crate::ui::menu::MenuItem> {
    use crate::ui::menu::MenuItem;
    vec![
        MenuItem::icon(SEND_KEY, Icon::Send, controls::SEND_RECEIPT),
        MenuItem::icon(DECLINE_KEY, Icon::X, controls::DONT_SEND_RECEIPT),
    ]
}

/// What the answer a menu key stands for is, if it is one of the two.
pub(in crate::ui) fn answer_of(key: &str) -> Option<ReceiptAnswer> {
    match key {
        SEND_KEY => Some(ReceiptAnswer::Sent),
        DECLINE_KEY => Some(ReceiptAnswer::Declined),
        _ => None,
    }
}

/// The request `message` puts to the person, if it puts one.
pub(in crate::ui) fn question_of(message: MessageId) -> Option<Question> {
    let Asks(asks) = try_consume_context::<Asks>()?;
    asks.read()
        .iter()
        .find(|ask| ask.message == message)
        .cloned()
}

/// Where a receipt would go when that is not plainly back to the sender: the line a popover
/// and a menu say under their rows.
pub(in crate::ui) fn destination_note(question: &Question) -> Option<String> {
    match &question.destination {
        Destination::Agreed => None,
        Destination::Elsewhere(to) => Some(status::receipt_goes_elsewhere(to)),
        Destination::Unchecked(to) => Some(status::receipt_goes_unchecked(to)),
    }
}

/// Give `answer` to the request of `message`: queue the receipt, or decline it, off the thread
/// that draws, and take the mark away once it is kept, not before: taking it away first would
/// end this component, and the task with it, before the answer was given. A second press
/// meanwhile is refused by `answer`, which finds the request answered.
pub(in crate::ui) fn give(message: MessageId, answer: ReceiptAnswer) {
    let looks =
        try_consume_context::<std::sync::Arc<Looks>>().unwrap_or_else(crate::ui::reading::looks);
    let client = crate::ui::live::client();
    let asks = try_consume_context::<Asks>();
    spawn(async move {
        let kept = tokio::task::spawn_blocking(move || {
            answer_one(&looks, &client, message, answer, Utc::now())
        })
        .await
        .unwrap_or(false);
        if !kept {
            // The mark stays, so the person can answer again, and the failure is said.
            crate::ui::motion::tell(
                crate::ui::copy::errors::RECEIPT_NOT_KEPT.to_owned(),
                crate::ui::motion::Follow::Nothing,
            );
            return;
        }
        if let Some(Asks(mut asks)) = asks {
            asks.write().retain(|ask| ask.message != message);
        }
    });
}

/// The small receipt mark in the header of a message that asks for a read receipt under
/// [`ReadReceipts::Ask`], and nothing for any other. Clicking it opens a popover with Send
/// Receipt and Don't Send. Ignoring it sends nothing.
#[component]
pub(in crate::ui) fn ReceiptMark(message: MessageId) -> Element {
    let mut open = use_signal(|| false);
    let mut at = use_signal(|| None::<MountedRef>);
    let Some(question) = question_of(message) else {
        return rsx! {};
    };
    rsx! {
        Button {
            bezel: Bezel::Inline,
            image: ImagePosition::Only,
            icon: Icon::MailOpen,
            label: tooltips::ASKS_FOR_A_RECEIPT,
            title: Some(tooltips::ASKS_FOR_A_RECEIPT.to_owned()),
            common: Common {
                id: Some(mark_id(message)),
                aria_label: Some(tooltips::ASKS_FOR_A_RECEIPT.to_owned()),
                mounted: Some(EventHandler::new(move |event: MountedEvent| at.set(Some(MountedRef(event.data()))))),
                ..Common::default()
            },
            onclick: on_primary(move || open.set(!open())),
        }
        if open() {
            super::menu::Floating {
                placement: MenuPlacement::Popup,
                anchor: at(),
                title: tooltips::ASKS_FOR_A_RECEIPT.to_owned(),
                items: items(),
                note: destination_note(&question),
                on_pick: move |key: String| {
                    open.set(false);
                    if let Some(answer) = answer_of(&key) {
                        give(message, answer);
                    }
                },
                on_close: move |()| open.set(false),
            }
        }
    }
}

/// The id of `message`'s receipt mark.
pub(in crate::ui) fn mark_id(message: MessageId) -> String {
    format!("receipt-mark-{message}")
}

#[cfg(test)]
mod tests;
