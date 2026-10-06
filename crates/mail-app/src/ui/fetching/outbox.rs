//! A send that has left its grace period wants a pass now, not at the next poll.
//!
//! The window queued a message and then waited for the poll, up to five minutes later, to
//! carry it: the outbox is drained by a full pass, and nothing started one when the grace
//! period ended. This is the decision of when one is wanted; the pill asks it on its tick.

use super::Fetching;
use chrono::{DateTime, Utc};
use mail_core::fetch::{Event, Link, Trigger};
use mail_domain::{DraftId, SendState};
use mail_store::{SqliteStore, Store};

/// Whether the pill should ask for a pass for the send of `draft`.
///
/// Once its time has come and the store still holds it unsent, and not while a pass is running:
/// that pass began before the message was due, so it may have gone by the outbox already, and
/// asking again would be ignored. The pill asks again on its next tick, so the request lands
/// the moment that pass ends. `asked` is the send this pill last asked for.
pub(in crate::ui) fn wants_a_pass(
    draft: DraftId,
    due: DateTime<Utc>,
    now: DateTime<Utc>,
    state: Option<&SendState>,
    link: Option<&Link>,
    asked: Option<DraftId>,
) -> bool {
    let waiting = matches!(state, Some(SendState::Queued | SendState::Scheduled { .. }));
    waiting && now >= due && asked != Some(draft) && link.is_some_and(|link| !link.is_busy())
}

impl Fetching {
    /// The pill's tick for the send of `draft`, due at `due`: ask for a pass if it is time, and
    /// remember in `asked` that it was asked, so one send is one request.
    pub(in crate::ui) fn drain_when_due(
        &self,
        store: &SqliteStore,
        draft: DraftId,
        due: DateTime<Utc>,
        now: DateTime<Utc>,
        asked: &mut Option<DraftId>,
    ) {
        let Ok(stored) = store.draft(draft) else {
            return;
        };
        let link = self.link(stored.account.clone());
        if wants_a_pass(draft, due, now, Some(&stored.state), link.as_ref(), *asked) {
            self.send(stored.account, Event::Start(Trigger::Manual));
            *asked = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests;
