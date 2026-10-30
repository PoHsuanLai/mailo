//! Calendar invitations in the reader: a card at the top of the message that carries one, the
//! three answers, and saving the event for a calendar.
//!
//! Whether a message invites is read from its stored raw bytes, so finding out reads a blob.
//! That is done off the thread that draws, for the messages the reader has open and nowhere
//! else — it is reached from [`draw::Invitation`] only, and a test counts the reads of the app's
//! [`Looks`](mail_client::message::Looks) — and the answer is kept per message and body there, like
//! the receipt bar's. A message whose body has not
//! been fetched shows no card: fetching it to find out would be a POP3 `RETR`, which marks it
//! read.
//!
//! What an invitation says and what answering does are [`mail_client::invite`]'s, the module
//! `mailo invite` uses, so the window and the command cannot disagree. Opening a message answers
//! nothing: an automatic "accepted" would tell every sender that this mailbox reads its mail, so
//! the only path to [`mail_client::invite::answer`] here is a press of Send.

mod card;
mod draw;

/// The card for an invitation, which Peek draws for a calendar file as well.
pub(in crate::ui) use card::card_of;
use card::{CHIPS, Card, Stand};
pub(in crate::ui) use draw::Invitation;

use chrono::{DateTime, Utc};
use mail_client::Client;
use mail_client::message::{Invited, Looks};
use mail_client::when::Clock;
use mail_domain::*;
use std::path::Path;

/// The card for what a message was found to invite to, with times in the reader's zone by
/// their `clock`.
fn card_from(message: MessageId, invited: &Invited, clock: Clock) -> Card {
    card_of(
        message,
        &invited.invite,
        invited.answered.as_ref(),
        &chrono::Local,
        clock,
    )
}

/// The card already found for `message` holding `body`, if it has been looked at. `Some(None)`
/// is a message that was looked at and invites to nothing.
pub(in crate::ui) fn cached(
    looks: &Looks,
    message: MessageId,
    body: Option<BlobId>,
    clock: Clock,
) -> Option<Option<Card>> {
    looks
        .invite_cached(message, body)
        .map(|invited| invited.map(|invited| card_from(message, &invited, clock)))
}

/// [`cached`], else read from the client and remembered. `None` when the message carries no
/// invitation, or only its headers are here. This reads a blob: it runs on a blocking thread,
/// for a message the reader has open, and nowhere else.
pub(in crate::ui) fn lookup(
    looks: &Looks,
    client: &Client,
    message: MessageId,
    body: Option<BlobId>,
    clock: Clock,
) -> Option<Card> {
    client
        .looking(looks)
        .invite(message, body)
        .map(|invited| card_from(message, &invited, clock))
}

/// Answer `message`'s invitation through [`mail_client::invite::answer`], and read its card back so
/// the reader shows what the client now holds. Blocking: the window calls it on a blocking
/// thread, and only from Send.
///
/// Returns what the toast says — the first line of the command's answer, which names who the
/// answer goes to; the rest tells a terminal to run `mailo sync`, which the window does itself —
/// and the card as it now stands.
pub(in crate::ui) fn answer(
    looks: &Looks,
    client: &Client,
    message: MessageId,
    attendance: Attendance,
    note: Option<&str>,
    now: DateTime<Utc>,
    clock: Clock,
) -> Result<(String, Option<Card>), String> {
    let answered = client.invite().answer(message, attendance, note, now)?;
    let said = mail_os::said::invite::said(&answered);
    let card = client
        .looking(looks)
        .invite_again(message)
        .map(|invited| card_from(message, &invited, clock));
    Ok((toast_text(&said), card))
}

/// Write `message`'s calendar object into `dir` as an `.ics` file named for the event, never
/// over a file already there. Returns what the toast says: where it went.
pub(in crate::ui) fn save_ics(
    client: &Client,
    message: MessageId,
    title: &str,
    dir: &Path,
) -> Result<std::path::PathBuf, String> {
    let bytes = client.invite().export(message)?;
    let stem = match title.trim() {
        "" | "(no title)" => "invitation",
        named => named,
    };
    mail_client::attach::write_new(dir, &format!("{stem}.ics"), &bytes).map_err(String::from)
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
pub(in crate::ui) mod card_tests;
#[cfg(test)]
mod tests;
