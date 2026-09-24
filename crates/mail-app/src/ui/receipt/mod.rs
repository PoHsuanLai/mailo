//! Read receipts in the reader: a message that asks is shown asking, and answered only by a
//! button pressed for that purpose.
//!
//! Whether a message asks is a header the domain message does not carry, so finding out reads
//! the stored raw message. That is done off the thread that draws, for the thread the reader has
//! open and nowhere else — [`look`] is reached from [`bar::Receipts`] only, and a test counts its
//! calls — and the answer is kept per message and body, like the list lookup beside it in the
//! head. A message whose body has not been fetched is [`ReceiptState::Unknown`] and shows
//! nothing: fetching it to find out would be a POP3 `RETR`, which marks it read.
//!
//! What asking and answering mean is [`crate::receipt`]'s, the module `mailo receipt` uses, so the
//! window and the command cannot disagree about it. Opening a message never answers: RFC 8098
//! §2.1 leaves that to the person reading, and this module has no path that answers without a
//! press of Send receipt or Don't send.

mod bar;

pub(super) use bar::Receipts;

use crate::receipt::ReceiptState;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::ReturnPath;
use mail_store::{SqliteStore, Store};

/// One message's standing, with the name the bar calls its sender by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Standing {
    pub message: MessageId,
    /// The sender's name, else their address.
    pub sender: String,
    pub state: ReceiptState,
}

/// What the bar says about one message, worked out from its standing. Pure, so every wording is
/// a table test away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Line {
    /// It asks. `warning` is set when the receipt would not go back where the message came from.
    Asking {
        sentence: String,
        warning: Option<String>,
    },
    /// Answered on this machine: a quiet note, no buttons.
    Settled(&'static str),
}

/// The line for `standing`, or `None` when there is nothing to say.
///
/// An answered message keeps a one-line note rather than going silent. The choice is between
/// two calm states, and the note is the one that answers the question a person has when they
/// reopen a message that once asked — "did I send one?" — without a trip to the command line;
/// it is one line of faint text with nothing to press, which is as quiet as a line can be.
pub(in crate::ui) fn line(standing: &Standing) -> Option<Line> {
    match &standing.state {
        ReceiptState::NotAsked | ReceiptState::Unknown => None,
        ReceiptState::Answered(ReceiptAnswer::Sent) => Some(Line::Settled("Receipt sent")),
        ReceiptState::Answered(ReceiptAnswer::Declined) => Some(Line::Settled("Receipt declined")),
        ReceiptState::Pending(ask) => {
            let to: Vec<&str> = ask.to.iter().map(|a| a.email.as_str()).collect();
            // The receipt goes to the `Disposition-Notification-To` addresses. `Differs` means
            // they are not on the domain the message came back from, which is the thing to see
            // before saying yes: anyone can write that header.
            let warning = match &ask.return_path {
                ReturnPath::Differs { return_path } => Some(format!(
                    "The receipt would go to {}, not to {return_path}, where this message came from.",
                    to.join(", ")
                )),
                ReturnPath::Agrees | ReturnPath::Unknown => None,
            };
            Some(Line::Asking {
                sentence: format!(
                    "{} asked to be told when you've read this.",
                    standing.sender
                ),
                warning,
            })
        }
    }
}

/// What a message's standing is a function of: the message and the bytes it has. A body that
/// arrives is a new key. An answer given here replaces the kept standing directly.
pub(in crate::ui) type Bodies = Vec<(MessageId, Option<BlobId>)>;

#[cfg(test)]
static LOOKED: std::sync::Mutex<Vec<MessageId>> = std::sync::Mutex::new(Vec::new());

/// Whether [`look`] has read `message`'s standing in this test process.
#[cfg(test)]
pub(in crate::ui) fn looked_at(message: MessageId) -> bool {
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .contains(&message)
}

/// Read `message`'s standing from the store. This may read a blob: it runs on a blocking
/// thread, for the thread the reader has open, and nowhere else.
pub(in crate::ui) fn look(store: &SqliteStore, message: MessageId) -> Option<Standing> {
    #[cfg(test)]
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .push(message);
    let stored = store.message(message).ok()?;
    let state = crate::receipt::state(store, &stored).ok()?;
    let sender = stored
        .from
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| stored.from.email.clone());
    Some(Standing {
        message,
        sender,
        state,
    })
}

/// How many messages' standings to keep. Each is a few short strings.
const KEPT: usize = 256;

type Kept = (MessageId, Option<BlobId>, Option<Standing>);

static CACHE: std::sync::Mutex<Vec<Kept>> = std::sync::Mutex::new(Vec::new());

/// The standings already found for every message in `key`, or `None` if any is missing.
pub(in crate::ui) fn cached(key: &Bodies) -> Option<Vec<Standing>> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    key.iter()
        .map(|(message, body)| {
            cache
                .iter()
                .find(|(had, at, _)| had == message && at == body)
                .map(|(_, _, standing)| standing.clone())
        })
        .collect::<Option<Vec<_>>>()
        .map(|all| all.into_iter().flatten().collect())
}

fn keep(message: MessageId, body: Option<BlobId>, standing: Option<Standing>) {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(had, _, _)| *had != message);
    cache.push((message, body, standing));
    if cache.len() > KEPT {
        cache.remove(0);
    }
}

/// [`cached`], else [`look`] for each message, remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(store: &SqliteStore, key: &Bodies) -> Vec<Standing> {
    if let Some(had) = cached(key) {
        return had;
    }
    key.iter()
        .filter_map(|(message, body)| {
            let standing = look(store, *message);
            keep(*message, *body, standing.clone());
            standing
        })
        .collect()
}

/// Answer `message`'s request through [`crate::receipt::answer`], and keep the answer as its
/// standing so the bar settles without reading the store again. Blocking: the window calls it
/// on a blocking thread, and only from a button.
///
/// Returns what the toast says: the first line of the command's answer, which names where the
/// receipt went. The rest of that answer tells a terminal to run `mailo sync`, which the window
/// does itself.
pub(in crate::ui) fn answer(
    store: &SqliteStore,
    message: MessageId,
    answer: ReceiptAnswer,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let said = crate::receipt::answer(store, message, answer, now)?;
    // Read back rather than assumed: the store's answer is the one the command will show too.
    if let Ok(stored) = store.message(message) {
        keep(message, stored.body.raw(), look(store, message));
    }
    Ok(toast_text(&said))
}

/// The first line of `said`, starting with a capital.
fn toast_text(said: &str) -> String {
    let first = said.lines().next().unwrap_or_default().trim();
    let mut chars = first.chars();
    match chars.next() {
        Some(head) => head.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
