//! Read receipts in the reader: a message that asks is shown asking, and answered only by a
//! button pressed for that purpose.
//!
//! Whether a message asks is a header the domain message does not carry, so finding out reads
//! the stored raw message. That is done off the thread that draws, for the thread the reader has
//! open and nowhere else — it is reached from [`bar::Receipts`] only, and a test counts the reads
//! of the app's [`Looks`] — and the answer is kept per message and body there, like the list
//! lookup beside it in the head. A message whose body has not been fetched is
//! [`ReceiptState::Unknown`] and shows nothing: fetching it to find out would be a POP3 `RETR`,
//! which marks it read.
//!
//! What asking and answering mean is [`mail_core::receipt`]'s, the module `mailo receipt` uses, so the
//! window and the command cannot disagree about it. Opening a message never answers: RFC 8098
//! §2.1 leaves that to the person reading, and this module has no path that answers without a
//! press of Send receipt or Don't send.

mod bar;

pub(super) use bar::Receipts;

use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::message::Looks;
use mail_core::receipt::{ReceiptState, Settled};
use mail_domain::*;
use mail_mime::ReturnPath;

// One message's standing, with the name the bar calls its sender by.
pub(in crate::ui) use mail_core::message::Receipt as Standing;

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
                    "The receipt goes to {}, not {return_path}.",
                    to.join(", ")
                )),
                ReturnPath::Agrees | ReturnPath::Unknown => None,
            };
            Some(Line::Asking {
                sentence: format!("{} asked for a read receipt.", standing.sender),
                warning,
            })
        }
    }
}

// What a thread's standings are a function of: the messages and the bytes each one has. A body
// that arrives is a new key. An answer given here replaces the kept standing directly.
pub(in crate::ui) use mail_core::message::Bodies;

/// The standings already found for every message in `key`, or `None` if any is missing.
pub(in crate::ui) fn cached(looks: &Looks, key: &Bodies) -> Option<Vec<Standing>> {
    looks.receipts_cached(key)
}

/// [`cached`], else read for each message and remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(looks: &Looks, store: &SqliteStore, key: &Bodies) -> Vec<Standing> {
    looks.receipts(store, key)
}

/// Answer `message`'s request through [`mail_core::receipt::answer`], and keep the answer as its
/// standing so the bar settles without reading the store again. Blocking: the window calls it
/// on a blocking thread, and only from a button.
///
/// Returns what the toast says: where the receipt went, or that none will. The command line
/// adds a line telling a terminal to run `mailo sync`, which the window does itself.
pub(in crate::ui) fn answer(
    looks: &Looks,
    store: &SqliteStore,
    message: MessageId,
    answer: ReceiptAnswer,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let settled = mail_core::receipt::answer(store, message, answer, now)?;
    // Read back rather than assumed: the store's answer is the one the command will show too.
    looks.receipt_again(store, message);
    Ok(match settled {
        Settled::Sent { to } => format!("Queued a read receipt to {}", to.join(", ")),
        Settled::Declined { to } => {
            format!("Declined: no receipt will go to {}", to.join(", "))
        }
    })
}

#[cfg(test)]
mod tests;
