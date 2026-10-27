//! A draft's life on the page, against the store: load, save, park, send, and take a send back.
//!
//! Every write goes through `mail_core::compose`, the module the command line uses, so the window and
//! `mailo send` cannot disagree about what a draft is.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::SigningStore;
use mail_core::compose::{Leaves, PictureSize};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::page::List;
use super::page::{Guard, Page, Phase, Saved, When, Wire};
use super::protection::Protection;
use super::recipients::commit_typed;
use super::seal::SealBar;
use crate::settings::{ComposeSettings, MessageFormat};
use crate::ui::today::Today;
use mail_core::editor::missing_attachment;

/// How long a send waits in the outbox before it may leave, which is how long Undo has: the
/// person's `compose.undo_send`, read when Send is pressed, so a change in Settings counts from the
/// next send and never moves one already counting down. `Off` is no wait at all.
pub(in crate::ui) fn grace() -> chrono::TimeDelta {
    crate::ui::prefs::current().compose.undo_send.grace()
}

/// The page for `draft`: the parked copy when there is one, exactly as it was left, else a page
/// built from the stored draft, written as [`format_of`] says under `compose`.
pub(in crate::ui) fn load(
    store: &SqliteStore,
    draft: DraftId,
    parked: &mut Vec<Page>,
    compose: &ComposeSettings,
) -> Option<Page> {
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
    let stored = store.draft(draft).ok()?;
    let attached = mail_core::compose::attached_to(store, &stored);
    let inline = mail_core::compose::inline_parts(store, &stored);
    // Nobody is suggested until something is typed: the book is asked then, for that text.
    let mut page = Page::opened(&stored, Vec::new(), attached, &inline);
    super::signoff::adopt(store, &mut page);
    page.format = format_of(store, &stored, compose);
    Some(page)
}

/// What `draft` is written as. A draft that kept HTML was written rich and stays so. One with
/// none has not been written by a rich composer: a new one, or one saved as plain text. It is
/// written as `compose.format` says, or, for a reply while `compose.reply_same_format` is on, as
/// the message it answers was, when that message's body is here to say.
pub(in crate::ui) fn format_of(
    store: &SqliteStore,
    draft: &Draft,
    compose: &ComposeSettings,
) -> MessageFormat {
    if draft.html.is_some() {
        return MessageFormat::Rich;
    }
    let original = draft
        .in_reply_to
        .and_then(|answered| mail_core::compose::written_as(store, answered))
        .map(|written| match written {
            mail_core::compose::Written::Rich => MessageFormat::Rich,
            mail_core::compose::Written::Plain => MessageFormat::Plain,
        });
    compose.format_for(original)
}

/// Write the page to its draft. The dot goes clean only when the store took it.
pub(in crate::ui) fn save(
    store: &SqliteStore,
    page: &mut Page,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let edited = {
        let mut written = written();
        let edited = put(store, page, now)?;
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
    store: &SqliteStore,
    page: &Page,
    stamp: u64,
    now: DateTime<Utc>,
) -> Result<Option<Draft>, String> {
    let mut written = written();
    if written.get(&page.draft).is_some_and(|last| *last > stamp) {
        return Ok(None);
    }
    let edited = put(store, page, now)?;
    written.insert(page.draft, stamp);
    Ok(Some(edited))
}

/// The stamp of the page each draft was last written from. Held while a write is made, so writes
/// of one draft go one at a time and in the order their pages were taken.
fn written() -> MutexGuard<'static, BTreeMap<DraftId, u64>> {
    static WRITTEN: Mutex<BTreeMap<DraftId, u64>> = Mutex::new(BTreeMap::new());
    WRITTEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The picture size each draft was last queued to send at. The draft keeps its pictures as
/// attached and the size is the page's choice, so a send taken back and queued again
/// ([`pictures_queued`]) asks here rather than send at full size. Held for the process, as a page
/// is: after a restart a draft sends its pictures as attached, as it always has.
fn queued_at() -> MutexGuard<'static, BTreeMap<DraftId, PictureSize>> {
    static QUEUED: Mutex<BTreeMap<DraftId, PictureSize>> = Mutex::new(BTreeMap::new());
    QUEUED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How large the pictures of `draft`'s last queued send were to go.
pub(in crate::ui) fn pictures_queued(draft: DraftId) -> PictureSize {
    queued_at().get(&draft).copied().unwrap_or_default()
}

/// Discard `draft` from the store, as every discard in this process does: under the lock the
/// writes hold, so an autosave already writing finishes first, and recorded as written by a page
/// taken now, so an autosave of a page taken before the discard is refused ([`save_stamped`]'s
/// `None`) rather than writing the draft back.
///
/// The record stays: dropped, it would let that autosave through. A page taken after it, as
/// one is after the discard's Undo puts the draft back, is later and writes as ever.
pub(crate) fn discard(store: &SqliteStore, draft: DraftId) -> Result<String, mail_core::CoreError> {
    let mut written = written();
    let subject = mail_core::compose::discard(store, draft)?;
    written.insert(draft, stamp());
    Ok(subject)
}

/// How long a draft being written goes between copies kept on the server, at most one each: a
/// copy is an upload of the whole message, attachments and all, and the server copy is for
/// finishing the draft elsewhere, not a keystroke log. Closing or parking the page keeps one at
/// once, whenever the last was.
pub(in crate::ui) const KEEP_EVERY: chrono::TimeDelta = chrono::TimeDelta::seconds(30);

/// When each draft's copy was last queued for the server, so autosave asks for one no more often
/// than [`KEEP_EVERY`].
#[derive(Debug, Default)]
pub(in crate::ui) struct Kept(BTreeMap<DraftId, DateTime<Utc>>);

/// Whether an autosave asks for a copy, or the page is going and one is kept regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Keeping {
    /// Autosave's: at most one per [`KEEP_EVERY`].
    WhenDue,
    /// Closing or parking the page: now.
    Now,
}

impl Kept {
    /// Whether a copy of `draft` is to be queued at `now`, noting that it is when it is.
    pub(in crate::ui) fn due(
        &mut self,
        draft: DraftId,
        now: DateTime<Utc>,
        keeping: Keeping,
    ) -> bool {
        let due = keeping == Keeping::Now
            || self
                .0
                .get(&draft)
                .is_none_or(|last| now - *last >= KEEP_EVERY);
        if due {
            self.0.insert(draft, now);
        }
        due
    }
}

/// Queue the draft's copy for the server's Drafts mailbox, as `keeping` says
/// ([`mail_core::compose::keep_on_server`]), in place of one still waiting. Whether one was.
///
/// Under the lock the writes hold, so a copy is made of the draft as the last save left it, and
/// never of one a discard has just taken away.
pub(in crate::ui) fn keep_on_server(
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
    keeping: Keeping,
) -> Result<bool, String> {
    static KEPT: Mutex<Kept> = Mutex::new(Kept(BTreeMap::new()));
    let _written = written();
    let due = KEPT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .due(draft, now, keeping);
    if !due {
        return Ok(false);
    }
    match mail_core::compose::keep_on_server(store, draft, now) {
        Ok(queued) => Ok(queued),
        // Discarded or sent from somewhere else meanwhile: there is nothing to keep.
        Err(mail_core::CoreError::Store(mail_core::StoreError::NoDraft(_))) => Ok(false),
        Err(why) => Err(why.to_string()),
    }
}

fn put(store: &SqliteStore, page: &Page, now: DateTime<Utc>) -> Result<Draft, String> {
    let base = store.draft(page.draft).map_err(|e| e.to_string())?;
    let edited = page.apply_to(&base, now);
    mail_core::compose::save(store, &edited)?;
    Ok(edited)
}

/// Put the draft aside: saved, kept exactly in `parked`, and listed in Today. A draft nothing was
/// put in ([`Draft::untouched`]) is thrown away instead: closing a composer opened by mistake
/// leaves no draft behind.
pub(in crate::ui) fn park(
    store: &SqliteStore,
    mut page: Page,
    parked: &mut Vec<Page>,
    today: &mut Today,
    space: crate::ui::space::SpaceId,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let saved = save(store, &mut page, now);
    if saved.as_ref().is_ok_and(Draft::untouched) {
        parked.retain(|kept| kept.draft != page.draft);
        today.unpark(&page.draft);
        return discard(store, page.draft)
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    // Leaving the page keeps a copy on the server now, however recently autosave kept one.
    if saved.is_ok()
        && let Err(why) = keep_on_server(store, page.draft, now, Keeping::Now)
    {
        log::warn!("the draft's copy on the server was not queued: {why}");
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
    /// Queued. It may leave at `due`; until then Undo takes it back. `grace` is how long Undo was
    /// given, which the pill's ring counts down from.
    Queued {
        draft: DraftId,
        due: DateTime<Utc>,
        when: When,
        grace: chrono::TimeDelta,
    },
    /// Saved and cleared to go, and to be signed or encrypted, or have its pictures made smaller,
    /// first. That reads the keyring and may ask for a passphrase, or decodes every picture, so
    /// the caller does it off the thread that draws, with [`queue_reminding`]. `going` is when it
    /// leaves, the wait a send that leaves now was given when Send was pressed, and its pictures'
    /// size; `remind` is the reminder's time, if one was asked for.
    Sealing {
        going: Going,
        remind: Option<DateTime<Utc>>,
    },
}

/// How a queued send goes: when it leaves, the wait Undo is given before one that leaves now,
/// and how large its pictures are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct Going {
    pub leaves: Leaves,
    pub grace: chrono::TimeDelta,
    pub pictures: PictureSize,
}

/// Send, if the guards allow it: no recipients shakes the To row, a mentioned attachment with
/// nothing attached shows the warning bar once, and a draft OpenPGP or S/MIME cannot sign or
/// encrypt as it asks says why in the same bar. A plain draft is queued here, with `secrets` as
/// the keyring; a draft to be sealed comes back as [`Sent::Sealing`]. A send that leaves now waits
/// `grace` first ([`grace`]).
#[allow(clippy::too_many_arguments)]
pub(in crate::ui) fn send<Tz: TimeZone>(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    page: &mut Page,
    anyway: Anyway,
    grace: chrono::TimeDelta,
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
    // Larger than its server takes: said before Send, and held once, for Send Anyway or a
    // smaller picture size.
    if anyway == Anyway::No && super::sizing::over(page).is_some() {
        page.guard = Guard::TooLarge;
        return Ok(Sent::Stopped);
    }
    page.guard = Guard::Clear;
    // A time that has gone is refused before anything is written, in the page's words.
    let leaves = super::later::leaves(page.when, now, zone, clock)?;
    // And a reminder that would come before the message leaves, the same way.
    let leaving = match leaves {
        Leaves::Now => now + grace,
        Leaves::At(at) => at,
    };
    let remind = page.remind.due(leaving, zone, clock)?;
    let saved = save(store, page, now)?;
    if page.protection != Protection::None {
        page.seal_bar = super::seal::sealable(store, &saved, now)?;
        if page.seal_bar != SealBar::Clear {
            return Ok(Sent::Stopped);
        }
        page.seal_bar = SealBar::Sealing;
        return Ok(Sent::Sealing {
            going: Going {
                leaves,
                grace,
                pictures: page.pictures,
            },
            remind,
        });
    }
    page.seal_bar = SealBar::Clear;
    // Pictures made smaller are decoded and encoded again, which is no work for the thread that
    // draws: queued the way a sealed send is, with nothing to say while it is done.
    if page.pictures != PictureSize::Actual {
        return Ok(Sent::Sealing {
            going: Going {
                leaves,
                grace,
                pictures: page.pictures,
            },
            remind,
        });
    }
    let due = queue_reminding(
        store,
        secrets,
        &mail_core::pgp::no_passphrase,
        page.draft,
        Going {
            leaves,
            grace,
            pictures: PictureSize::Actual,
        },
        remind,
        now,
    )
    .map_err(crate::ui::remedy::told)?;
    Ok(folded(page, due, grace))
}

/// [`queue_reminding`] with no reminder, as the sealing tests queue.
#[cfg(test)]
pub(in crate::ui) fn queue(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    ask: mail_core::pgp::Ask<'_>,
    draft: DraftId,
    leaves: Leaves,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, mail_core::compose::SendError> {
    let going = Going {
        leaves,
        grace: chrono::TimeDelta::seconds(5),
        pictures: PictureSize::Actual,
    };
    queue_reminding(store, secrets, ask, draft, going, None, now)
}

/// Queue the saved draft to leave as `going.leaves` says, after `going.grace` when that is now,
/// signed and encrypted as it asks, with `ask` for its OpenPGP key's passphrase. Returns when it may leave, or why not, typed: a key that
/// stayed locked is [`mail_core::compose::SendError::locked`]. For a draft to be sealed this reads
/// the keyring, and the window calls it on a blocking thread.
///
/// `remind` is held as the message's follow-up reminder until it has left (`mail_core::follow_up`);
/// with none, a reminder an earlier send of the draft held goes. Its pictures go at `going.pictures`,
/// made smaller as the bytes are frozen unless that is [`PictureSize::Actual`].
pub(in crate::ui) fn queue_reminding(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    ask: mail_core::pgp::Ask<'_>,
    draft: DraftId,
    Going {
        leaves,
        grace,
        pictures,
    }: Going,
    remind: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, mail_core::compose::SendError> {
    let queue = |leaves, now| {
        mail_core::compose::queue_sized(store, secrets, ask, draft, leaves, pictures, now)
    };
    let (queued, post, due) = match leaves {
        // Right away is the grace period and Undo.
        Leaves::Now => {
            let due = now + grace;
            let (queued, post) = queue(Leaves::Now, due)?;
            (queued, post, due)
        }
        Leaves::At(at) => {
            let (queued, post) = queue(Leaves::At(at), now)?;
            (queued, post, at)
        }
    };
    match pictures {
        PictureSize::Actual => queued_at().remove(&draft),
        size => queued_at().insert(draft, size),
    };
    // The send is queued whatever becomes of the reminder: one that could not be kept is said
    // where the window says what it could not do, and is no reason to take back mail the outbox
    // already holds.
    if let Err(why) = mail_core::follow_up::after_queue(store, &queued, &post.message, remind, due)
    {
        log::warn!("the reminder for {draft} was not kept: {why}");
    }
    Ok(due)
}

/// The page once its draft is queued to leave at `due`, after `grace`: folding away.
pub(in crate::ui) fn folded(page: &mut Page, due: DateTime<Utc>, grace: chrono::TimeDelta) -> Sent {
    page.seal_bar = SealBar::Clear;
    page.phase = Phase::Folding;
    page.float = super::page::Float::Closed;
    Sent::Queued {
        draft: page.draft,
        due,
        when: page.when,
        grace,
    }
}

/// Undo a send: the queued submission is withdrawn and the draft is editable again.
pub(in crate::ui) fn unsend(
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    mail_core::compose::unsend(store, draft, now).map_err(crate::ui::remedy::told)
}
