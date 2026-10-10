//! Calendar invitations in the reader: a card at the top of the message that carries one, the
//! three answers, and saving the event for a calendar.
//!
//! Whether a message invites is read from its stored raw bytes, so finding out reads a blob.
//! That is done off the thread that draws, for the messages the reader has open and nowhere
//! else — it is reached from [`draw::Invitation`] only, and a test counts the reads of the app's
//! [`Looks`](mail_core::message::Looks) — and the answer is kept per message and body there, like
//! the receipt bar's. A message whose body has not
//! been fetched shows no card: fetching it to find out would be a POP3 `RETR`, which marks it
//! read.
//!
//! What an invitation says and what answering does are [`mail_core::invite`]'s, the module
//! `mailo invite` uses, so the window and the command cannot disagree. Opening a message answers
//! nothing: an automatic "accepted" would tell every sender that this mailbox reads its mail, so
//! the only path to [`mail_core::invite::answer`] here is a press of Send.

mod card;
mod draw;

use card::{CHIPS, Card, Stand, card_of};
pub(in crate::ui) use draw::Invitation;

use chrono::{DateTime, Utc};
use mail_core::SqliteStore;
use mail_core::message::{Invited, Looks};
use mail_domain::*;
use std::path::Path;

/// The card for what a message was found to invite to, with times in the reader's zone.
fn card_from(message: MessageId, invited: &Invited) -> Card {
    card_of(
        message,
        &invited.invite,
        invited.answered.as_ref(),
        &chrono::Local,
    )
}

/// The card already found for `message` holding `body`, if it has been looked at. `Some(None)`
/// is a message that was looked at and invites to nothing.
pub(in crate::ui) fn cached(
    looks: &Looks,
    message: MessageId,
    body: Option<BlobId>,
) -> Option<Option<Card>> {
    looks
        .invite_cached(message, body)
        .map(|invited| invited.map(|invited| card_from(message, &invited)))
}

/// [`cached`], else read from the store and remembered. `None` when the message carries no
/// invitation, or only its headers are here. This reads a blob: it runs on a blocking thread,
/// for a message the reader has open, and nowhere else.
pub(in crate::ui) fn lookup(
    looks: &Looks,
    store: &SqliteStore,
    message: MessageId,
    body: Option<BlobId>,
) -> Option<Card> {
    looks
        .invite(store, message, body)
        .map(|invited| card_from(message, &invited))
}

/// Answer `message`'s invitation through [`mail_core::invite::answer`], and read its card back so
/// the reader shows what the store now holds. Blocking: the window calls it on a blocking
/// thread, and only from Send.
///
/// Returns what the toast says — the first line of the command's answer, which names who the
/// answer goes to; the rest tells a terminal to run `mailo sync`, which the window does itself —
/// and the card as it now stands.
pub(in crate::ui) fn answer(
    looks: &Looks,
    store: &SqliteStore,
    message: MessageId,
    attendance: Attendance,
    note: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(String, Option<Card>), String> {
    let answered = mail_core::invite::answer(store, message, attendance, note, now)?;
    let said = crate::said::invite::said(&answered);
    let card = looks
        .invite_again(store, message)
        .map(|invited| card_from(message, &invited));
    Ok((toast_text(&said), card))
}

/// Write `message`'s calendar object into `dir` as an `.ics` file named for the event, never
/// over a file already there. Returns what the toast says: where it went.
pub(in crate::ui) fn save_ics(
    store: &SqliteStore,
    message: MessageId,
    title: &str,
    dir: &Path,
) -> Result<std::path::PathBuf, String> {
    let bytes = mail_core::invite::export(store, message)?;
    let stem = match title.trim() {
        "" | "(no title)" => "invitation",
        named => named,
    };
    mail_core::attach::write_new(dir, &format!("{stem}.ics"), &bytes).map_err(String::from)
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
mod card_tests;
#[cfg(test)]
mod tests;
