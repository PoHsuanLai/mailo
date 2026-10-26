//! A draft's life on the page, against the client: load, save, park, send, and take a send back.
//!
//! Every write goes through `mail_core::compose`, the module the command line uses, so the window and
//! `mailo send` cannot disagree about what a draft is.

use chrono::{DateTime, TimeZone, Utc};
use mail_client::Client;
use mail_core::SigningStore;
use mail_core::compose::Leaves;
use mail_domain::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::page::List;
use super::page::{Guard, Page, Phase, Saved, When, Wire};
use super::protection::Protection;
use super::recipients::commit_typed;
use super::seal::SealBar;
use crate::ui::today::Today;
use mail_core::editor::missing_attachment;

/// How long a send waits in the outbox before it may leave, which is how long Undo has.
pub(in crate::ui) const GRACE: chrono::TimeDelta = chrono::TimeDelta::seconds(5);

/// The page for `draft`: the parked copy when there is one, exactly as it was left, else a page
/// built from the stored draft.
pub(in crate::ui) fn load(client: &Client, draft: DraftId, parked: &mut Vec<Page>) -> Option<Page> {
    if let Some(at) = parked.iter().position(|page| page.draft == draft) {
        let mut page = parked.remove(at);
        page.phase = Phase::Writing;
        // A new body element numbers the surface's input from zero again.
        page.wire = Wire {
            seq: 0,
            composing: None,
        };
        return Some(page);
    }
    let stored = client.stored().draft(draft).ok()?;
    let attached = client.compose().attached_to(&stored);
    // Nobody is suggested until something is typed: the book is asked then, for that text.
    Some(Page::of(&stored, Vec::new(), attached))
}

/// Write the page to its draft. The dot goes clean only when the client took it.
pub(in crate::ui) fn save(
    client: &Client,
    page: &mut Page,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let edited = {
        let mut written = written();
        let edited = put(client, page, now)?;
        // This page is the newest there is: nothing was taken after it.
        written.insert(page.draft, stamp());
        edited
    };
    page.saved = Saved::Clean;
    Ok(edited)
}

/// A page taken now, to be written by [`save_stamped`]: each is later than every one before.
pub(in crate::ui) fn stamp() -> u64 {
    static STAMPS: AtomicU64 = AtomicU64::new(0);
    STAMPS.fetch_add(1, Ordering::Relaxed) + 1
}

/// Write `page`, as it was when `stamp` was taken, to its draft, off the thread that draws:
/// autosave's. `None` when a page taken later was written first, by a save of the page's own,
/// which a page taken earlier must not undo. The page's dot is the caller's to clear.
pub(in crate::ui) fn save_stamped(
    client: &Client,
    page: &Page,
    stamp: u64,
    now: DateTime<Utc>,
) -> Result<Option<Draft>, String> {
    let mut written = written();
    if written.get(&page.draft).is_some_and(|last| *last > stamp) {
        return Ok(None);
    }
    let edited = put(client, page, now)?;
    written.insert(page.draft, stamp);
    Ok(Some(edited))
}

/// The stamp of the page each draft was last written from. Held while a write is made, so writes
/// of one draft go one at a time and in the order their pages were taken.
fn written() -> MutexGuard<'static, BTreeMap<DraftId, u64>> {
    static WRITTEN: Mutex<BTreeMap<DraftId, u64>> = Mutex::new(BTreeMap::new());
    WRITTEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Discard `draft` from the client, as every discard in this process does: under the lock the
/// writes hold, so an autosave already writing finishes first, and recorded as written by a page
/// taken now, so an autosave of a page taken before the discard is refused ([`save_stamped`]'s
/// `None`) rather than writing the draft back.
///
/// The record stays: dropped, it would let that autosave through. A page taken after it, as
/// one is after the discard's Undo puts the draft back, is later and writes as ever.
pub(crate) fn discard(client: &Client, draft: DraftId) -> Result<String, mail_core::CoreError> {
    discarding(draft, || client.compose().discard(draft))
}

/// [`discard`], made by `discard`: for the program beside the window, which holds the store
/// itself (`intents`).
pub(crate) fn discarding<T, E>(
    draft: DraftId,
    discard: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let mut written = written();
    let done = discard()?;
    written.insert(draft, stamp());
    Ok(done)
}

fn put(client: &Client, page: &Page, now: DateTime<Utc>) -> Result<Draft, String> {
    let base = client
        .stored()
        .draft(page.draft)
        .map_err(|e| e.to_string())?;
    let edited = page.apply_to(&base, now);
    client.compose().save(&edited)?;
    Ok(edited)
}

/// Put the draft aside: saved, kept exactly in `parked`, and listed in Today. A draft nothing was
/// put in ([`Draft::untouched`]) is thrown away instead: closing a composer opened by mistake
/// leaves no draft behind.
pub(in crate::ui) fn park(
    client: &Client,
    mut page: Page,
    parked: &mut Vec<Page>,
    today: &mut Today,
    space: crate::ui::space::SpaceId,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let saved = save(client, &mut page, now);
    if saved.as_ref().is_ok_and(Draft::untouched) {
        parked.retain(|kept| kept.draft != page.draft);
        today.unpark(&page.draft);
        return discard(client, page.draft)
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let title = if page.subject.trim().is_empty() {
        "Draft".to_owned()
    } else {
        page.subject.clone()
    };
    page.float = super::page::Float::Closed;
    page.selection = None;
    parked.retain(|kept| kept.draft != page.draft);
    today.park(space, page.draft, &title, crate::ui::today::at(now));
    parked.push(page);
    saved.map(|_| ())
}

/// Whether the warning bar's "Send anyway" was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Anyway {
    No,
    Yes,
}

/// What a press of Send did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Sent {
    /// A guard stopped it. Nothing was queued.
    Stopped,
    /// Queued. It may leave at `due`; until then Undo takes it back.
    Queued {
        draft: DraftId,
        due: DateTime<Utc>,
        when: When,
    },
    /// Saved and cleared to go, and to be signed or encrypted first. That reads the keyring and
    /// may ask for a passphrase, so the caller does it off the thread that draws, with
    /// [`queue_reminding`]. `remind` is the reminder's time, if one was asked for.
    Sealing {
        leaves: Leaves,
        remind: Option<DateTime<Utc>>,
    },
}

/// Send, if the guards allow it: no recipients shakes the To row, a mentioned attachment with
/// nothing attached shows the warning bar once, and a draft OpenPGP or S/MIME cannot sign or
/// encrypt as it asks says why in the same bar. A plain draft is queued here, with `secrets` as
/// the keyring; a draft to be sealed comes back as [`Sent::Sealing`].
pub(in crate::ui) fn send<Tz: TimeZone>(
    client: &Client,
    secrets: &dyn SigningStore,
    page: &mut Page,
    anyway: Anyway,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: mail_core::when::Clock,
) -> Result<Sent, String>
where
    Tz::Offset: std::fmt::Display,
{
    if !commit_typed(page, List::To)
        || !commit_typed(page, List::Cc)
        || !commit_typed(page, List::Bcc)
    {
        return Ok(Sent::Stopped);
    }
    if page.to.is_empty() && page.cc.is_empty() && page.bcc.is_empty() {
        let count = match page.guard {
            Guard::NoRecipient(count) => count + 1,
            _ => 1,
        };
        page.guard = Guard::NoRecipient(count);
        return Ok(Sent::Stopped);
    }
    if anyway == Anyway::No && page.attached.is_empty() && missing_attachment(&page.session.doc) {
        page.guard = Guard::Warn;
        return Ok(Sent::Stopped);
    }
    page.guard = Guard::Clear;
    // A time that has gone is refused before anything is written, in the page's words.
    let leaves = super::later::leaves(page.when, now, zone, clock)?;
    // And a reminder that would come before the message leaves, the same way.
    let leaving = match leaves {
        Leaves::Now => now + GRACE,
        Leaves::At(at) => at,
    };
    let remind = page.remind.due(leaving, zone, clock)?;
    let saved = save(client, page, now)?;
    if page.protection != Protection::None {
        page.seal_bar = super::seal::sealable(client, &saved, now)?;
        if page.seal_bar != SealBar::Clear {
            return Ok(Sent::Stopped);
        }
        page.seal_bar = SealBar::Sealing;
        return Ok(Sent::Sealing { leaves, remind });
    }
    page.seal_bar = SealBar::Clear;
    let due = queue_reminding(
        client,
        secrets,
        &mail_core::pgp::no_passphrase,
        page.draft,
        leaves,
        remind,
        now,
    )
    .map_err(crate::ui::remedy::told)?;
    Ok(folded(page, due))
}

/// [`queue_reminding`] with no reminder, as the sealing tests queue.
#[cfg(test)]
pub(in crate::ui) fn queue(
    client: &Client,
    secrets: &dyn SigningStore,
    ask: mail_core::pgp::Ask<'_>,
    draft: DraftId,
    leaves: Leaves,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, mail_core::compose::SendError> {
    queue_reminding(client, secrets, ask, draft, leaves, None, now)
}

/// Queue the saved draft to leave as `leaves` says, signed and encrypted as it asks, with `ask`
/// for its OpenPGP key's passphrase. Returns when it may leave, or why not, typed: a key that
/// stayed locked is [`mail_core::compose::SendError::locked`]. For a draft to be sealed this reads
/// the keyring, and the window calls it on a blocking thread.
///
/// `remind` is held as the message's follow-up reminder until it has left (`mail_core::follow_up`);
/// with none, a reminder an earlier send of the draft held goes.
pub(in crate::ui) fn queue_reminding(
    client: &Client,
    secrets: &dyn SigningStore,
    ask: mail_core::pgp::Ask<'_>,
    draft: DraftId,
    leaves: Leaves,
    remind: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, mail_core::compose::SendError> {
    let (queued, post, due) = match leaves {
        // Right away is the grace period and Undo, as it always was.
        Leaves::Now => {
            let due = now + GRACE;
            let (queued, post) =
                client
                    .compose()
                    .queue_with(secrets, ask, draft, Leaves::Now, due)?;
            (queued, post, due)
        }
        Leaves::At(at) => {
            let (queued, post) =
                client
                    .compose()
                    .queue_with(secrets, ask, draft, Leaves::At(at), now)?;
            (queued, post, at)
        }
    };
    // The send is queued whatever becomes of the reminder: one that could not be kept is said
    // where the window says what it could not do, and is no reason to take back mail the outbox
    // already holds.
    if let Err(why) = client
        .follow_up()
        .after_queue(&queued, &post.message, remind, due)
    {
        log::warn!("the reminder for {draft} was not kept: {why}");
    }
    Ok(due)
}

/// The page once its draft is queued to leave at `due`: folding away.
pub(in crate::ui) fn folded(page: &mut Page, due: DateTime<Utc>) -> Sent {
    page.seal_bar = SealBar::Clear;
    page.phase = Phase::Folding;
    page.float = super::page::Float::Closed;
    Sent::Queued {
        draft: page.draft,
        due,
        when: page.when,
    }
}

/// Undo a send: the queued submission is withdrawn and the draft is editable again.
pub(in crate::ui) fn unsend(
    client: &Client,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    client
        .compose()
        .unsend(draft, now)
        .map_err(crate::ui::remedy::told)
}
