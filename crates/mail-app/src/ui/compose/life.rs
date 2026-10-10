//! A draft's life on the page, against the store: load, save, park, send, and take a send back.
//!
//! Every write goes through `mail_core::compose`, the module the command line uses, so the window and
//! `mailo send` cannot disagree about what a draft is.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::compose::Leaves;
use mail_domain::*;
use mail_runtime::SigningStore;
use mail_store::{SqliteStore, Store};

use super::page::List;
use super::page::{Guard, Page, Phase, Saved, When, Wire};
use super::protection::Protection;
use super::recipients::commit_typed;
use super::seal::SealBar;
use crate::ui::editor::missing_attachment;
use crate::ui::today::Today;

/// How long a send waits in the outbox before it may leave, which is how long Undo has.
pub(in crate::ui) const GRACE: chrono::TimeDelta = chrono::TimeDelta::seconds(5);

/// The page for `draft`: the parked copy when there is one, exactly as it was left, else a page
/// built from the stored draft.
pub(in crate::ui) fn load(
    store: &SqliteStore,
    draft: DraftId,
    parked: &mut Vec<Page>,
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
    // Nobody is suggested until something is typed: the book is asked then, for that text.
    Some(Page::of(&stored, Vec::new(), attached))
}

/// Write the page to its draft. The dot goes clean only when the store took it.
pub(in crate::ui) fn save(
    store: &SqliteStore,
    page: &mut Page,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let base = store.draft(page.draft).map_err(|e| e.to_string())?;
    let edited = page.apply_to(&base, now);
    mail_core::compose::save(store, &edited)?;
    page.saved = Saved::Clean;
    Ok(edited)
}

/// Put the draft aside: saved, kept exactly in `parked`, and listed in Today.
pub(in crate::ui) fn park(
    store: &SqliteStore,
    mut page: Page,
    parked: &mut Vec<Page>,
    today: &mut Today,
    space: crate::ui::space::SpaceId,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let saved = save(store, &mut page, now);
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
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    page: &mut Page,
    anyway: Anyway,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Sent, String>
where
    Tz::Offset: std::fmt::Display,
{
    if !commit_typed(page, List::To) || !commit_typed(page, List::Cc) {
        return Ok(Sent::Stopped);
    }
    if page.to.is_empty() && page.cc.is_empty() {
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
    let leaves = super::later::leaves(page.when, now, zone)?;
    // And a reminder that would come before the message leaves, the same way.
    let leaving = match leaves {
        Leaves::Now => now + GRACE,
        Leaves::At(at) => at,
    };
    let remind = page.remind.due(leaving, zone)?;
    let saved = save(store, page, now)?;
    if page.protection != Protection::None {
        page.seal_bar = super::seal::sealable(store, &saved, now)?;
        if page.seal_bar != SealBar::Clear {
            return Ok(Sent::Stopped);
        }
        page.seal_bar = SealBar::Sealing;
        return Ok(Sent::Sealing { leaves, remind });
    }
    page.seal_bar = SealBar::Clear;
    let due = queue_reminding(
        store,
        secrets,
        &mail_core::pgp::no_passphrase,
        page.draft,
        leaves,
        remind,
        now,
    )
    .map_err(|refused| refused.to_string())?;
    Ok(folded(page, due))
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
    queue_reminding(store, secrets, ask, draft, leaves, None, now)
}

/// Queue the saved draft to leave as `leaves` says, signed and encrypted as it asks, with `ask`
/// for its OpenPGP key's passphrase. Returns when it may leave, or why not, typed: a key that
/// stayed locked is [`mail_core::compose::SendError::locked`]. For a draft to be sealed this reads
/// the keyring, and the window calls it on a blocking thread.
///
/// `remind` is held as the message's follow-up reminder until it has left (`mail_core::follow_up`);
/// with none, a reminder an earlier send of the draft held goes.
pub(in crate::ui) fn queue_reminding(
    store: &SqliteStore,
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
                mail_core::compose::queue_with(store, secrets, ask, draft, Leaves::Now, due)?;
            (queued, post, due)
        }
        Leaves::At(at) => {
            let (queued, post) =
                mail_core::compose::queue_with(store, secrets, ask, draft, Leaves::At(at), now)?;
            (queued, post, at)
        }
    };
    // The send is queued whatever becomes of the reminder: one that could not be kept is said
    // where the window says what it could not do, and is no reason to take back mail the outbox
    // already holds.
    if let Err(why) = mail_core::follow_up::after_queue(store, &queued, &post.message, remind, due)
    {
        eprintln!("the reminder for {draft} was not kept: {why}");
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
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    mail_core::compose::unsend(store, draft, now).map_err(String::from)
}
