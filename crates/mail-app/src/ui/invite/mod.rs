//! Calendar invitations in the reader: a card at the top of the message that carries one, the
//! three answers, and saving the event for a calendar.
//!
//! Whether a message invites is read from its stored raw bytes, so finding out reads a blob.
//! That is done off the thread that draws, for the messages the reader has open and nowhere
//! else — [`look`] is reached from [`draw::Invitation`] only, and a test counts its calls — and
//! the answer is kept per message and body, like the receipt bar's. A message whose body has not
//! been fetched shows no card: fetching it to find out would be a POP3 `RETR`, which marks it
//! read.
//!
//! What an invitation says and what answering does are [`crate::invite`]'s, the module
//! `mailo invite` uses, so the window and the command cannot disagree. Opening a message answers
//! nothing: an automatic "accepted" would tell every sender that this mailbox reads its mail, so
//! the only path to [`crate::invite::answer`] here is a press of Send.

mod card;
mod draw;

use card::{CHIPS, Card, Stand, card_of};
pub(in crate::ui) use draw::Invitation;

use crate::invite::InviteState;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::path::Path;

#[cfg(test)]
static LOOKED: std::sync::Mutex<Vec<MessageId>> = std::sync::Mutex::new(Vec::new());

/// Whether [`look`] has read `message` in this test process.
#[cfg(test)]
pub(in crate::ui) fn looked_at(message: MessageId) -> bool {
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .contains(&message)
}

/// Read `message`'s invitation from the store, with times in the reader's zone. `None` when it
/// carries none, or only its headers are here. This reads a blob: it runs on a blocking thread,
/// for a message the reader has open, and nowhere else.
pub(in crate::ui) fn look(store: &SqliteStore, message: MessageId) -> Option<Card> {
    #[cfg(test)]
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .push(message);
    let stored = store.message(message).ok()?;
    match crate::invite::state(store, &stored).ok()? {
        InviteState::Shown { invite, answered } => {
            Some(card_of(message, &invite, answered.as_ref(), &chrono::Local))
        }
        InviteState::NotInvite | InviteState::Unknown => None,
    }
}

/// How many messages' cards to keep. Each is a few short strings.
const KEPT: usize = 128;

type Kept = (MessageId, Option<BlobId>, Option<Card>);

static CACHE: std::sync::Mutex<Vec<Kept>> = std::sync::Mutex::new(Vec::new());

/// The card already found for `message` holding `body`, if it has been looked at. `Some(None)`
/// is a message that was looked at and invites to nothing.
pub(in crate::ui) fn cached(message: MessageId, body: Option<BlobId>) -> Option<Option<Card>> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(had, at, _)| *had == message && *at == body)
        .map(|(_, _, card)| card.clone())
}

fn keep(message: MessageId, body: Option<BlobId>, card: Option<Card>) {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(had, _, _)| *had != message);
    cache.push((message, body, card));
    if cache.len() > KEPT {
        cache.remove(0);
    }
}

/// [`cached`], else [`look`], remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(
    store: &SqliteStore,
    message: MessageId,
    body: Option<BlobId>,
) -> Option<Card> {
    if let Some(had) = cached(message, body) {
        return had;
    }
    let card = look(store, message);
    keep(message, body, card.clone());
    card
}

/// Answer `message`'s invitation through [`crate::invite::answer`], and read its card back so
/// the reader shows what the store now holds. Blocking: the window calls it on a blocking
/// thread, and only from Send.
///
/// Returns what the toast says — the first line of the command's answer, which names who the
/// answer goes to; the rest tells a terminal to run `mailo sync`, which the window does itself —
/// and the card as it now stands.
pub(in crate::ui) fn answer(
    store: &SqliteStore,
    message: MessageId,
    attendance: Attendance,
    note: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(String, Option<Card>), String> {
    let said = crate::invite::answer(store, message, attendance, note, now)?;
    let card = look(store, message);
    if let Ok(stored) = store.message(message) {
        keep(message, stored.body.raw(), card.clone());
    }
    Ok((toast_text(&said), card))
}

/// Write `message`'s calendar object into `dir` as an `.ics` file named for the event, never
/// over a file already there. Returns what the toast says: where it went.
pub(in crate::ui) fn save_ics(
    store: &SqliteStore,
    message: MessageId,
    title: &str,
    dir: &Path,
) -> Result<String, String> {
    let bytes = crate::invite::export(store, message)?;
    let stem = match title.trim() {
        "" | "(no title)" => "invitation",
        named => named,
    };
    let path = crate::attach::write_new(dir, &format!("{stem}.ics"), &bytes)?;
    Ok(format!("Saved to {}", path.display()))
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
