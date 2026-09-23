//! A draft's life on the page, against the store: load, save, park, send, and take a send back.
//!
//! Every write goes through `crate::compose`, the module the command line uses, so the window and
//! `mailo send` cannot disagree about what a draft is.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::page::List;
use super::page::{Guard, Page, Phase, Saved, When, Wire};
use super::recipients::commit_typed;
use crate::editor::{Person, missing_attachment};
use crate::today::Today;

/// How long a send waits in the outbox before it may leave, which is how long Undo has.
pub(in crate::ui) const GRACE: chrono::TimeDelta = chrono::TimeDelta::seconds(5);

/// How many recent conversations the people suggestions are drawn from.
const PEOPLE_FROM: u32 = 200;

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
        // A new body element numbers the glue's messages from zero again.
        page.wire = Wire {
            seq: 0,
            composing: None,
        };
        return Some(page);
    }
    let stored = store.draft(draft).ok()?;
    let attached = crate::compose::attached_to(store, &stored);
    Some(Page::of(&stored, people(store), attached))
}

/// The people this mailbox has written with: senders and participants of recent conversations,
/// less the mailbox's own addresses.
pub(in crate::ui) fn people(store: &SqliteStore) -> Vec<Person> {
    let own: Vec<String> = crate::compose::sending_accounts(store)
        .into_iter()
        .map(|(address, _)| address.to_ascii_lowercase())
        .collect();
    let query = Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: PEOPLE_FROM,
        },
    };
    let Ok(threads) = store.threads(&query, Utc::now()) else {
        return Vec::new();
    };
    let mut out: Vec<Person> = Vec::new();
    for summary in &threads.items {
        for address in std::iter::once(&summary.from).chain(&summary.participants) {
            let email = address.email.to_ascii_lowercase();
            if email.is_empty() || own.contains(&email) {
                continue;
            }
            if !out
                .iter()
                .any(|known| known.address.eq_ignore_ascii_case(&email))
            {
                out.push(super::page::person(address));
            }
        }
    }
    out
}

/// Write the page to its draft. The dot goes clean only when the store took it.
pub(in crate::ui) fn save(
    store: &SqliteStore,
    page: &mut Page,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let base = store.draft(page.draft).map_err(|e| e.to_string())?;
    let edited = page.apply_to(&base, now);
    crate::compose::save(store, &edited)?;
    page.saved = Saved::Clean;
    Ok(edited)
}

/// Put the draft aside: saved, kept exactly in `parked`, and listed in Today.
pub(in crate::ui) fn park(
    store: &SqliteStore,
    mut page: Page,
    parked: &mut Vec<Page>,
    today: &mut Today,
    space: usize,
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
    today.park(space, page.draft, &title, now);
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
}

/// Send, if the guards allow it: no recipients shakes the To row, and a mentioned attachment
/// with nothing attached shows the warning bar once.
pub(in crate::ui) fn send<Tz: TimeZone>(
    store: &SqliteStore,
    page: &mut Page,
    anyway: Anyway,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Sent, String> {
    if !commit_typed(page, List::To) || !commit_typed(page, List::Cc) {
        return Ok(Sent::Stopped);
    }
    if page.to.is_empty() && page.cc.is_empty() {
        let count = match page.guard {
            Guard::Shake(count) => count + 1,
            _ => 1,
        };
        page.guard = Guard::Shake(count);
        return Ok(Sent::Stopped);
    }
    if anyway == Anyway::No && page.attached.is_empty() && missing_attachment(&page.session.doc) {
        page.guard = Guard::Warn;
        return Ok(Sent::Stopped);
    }
    page.guard = Guard::Clear;
    save(store, page, now)?;
    let due = page.when.due(now, zone).unwrap_or(now + GRACE);
    crate::compose::send(store, page.draft, due)?;
    page.phase = Phase::Folding;
    page.float = super::page::Float::Closed;
    Ok(Sent::Queued {
        draft: page.draft,
        due,
        when: page.when,
    })
}

/// Undo a send: the queued submission is withdrawn and the draft is editable again.
pub(in crate::ui) fn unsend(
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    crate::compose::unsend(store, draft, now)
}
