//! Proposing another time for an invitation: the iTIP `COUNTER` (RFC 5546 §3.2.7) an attendee
//! sends the organiser, who may take the time (and send an update) or keep theirs.
//!
//! Like an answer it is mail to the organiser through the outbox, sent only when the person
//! presses Send, and from the identity the invitation names. Unlike an answer it changes nothing
//! on this side: the event stays as the organiser has it until the organiser says otherwise, so
//! no answer is recorded and no calendar is touched.

use super::{Asked, Itip, asked, one_line, product, queue, show_when};
use crate::error::CoreError;
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::MessageId;
use mail_pim::When;
use mail_pim::ical::{self, Proposing};
use mail_store::SqliteStore;

/// A proposal that is queued: who it goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    /// The envelope recipients of the queued message: the organiser.
    pub to: Vec<String>,
}

/// How long an event lasts when its invitation does not say: the hour calendars start with.
const AN_HOUR: TimeDelta = TimeDelta::hours(1);

/// How long the event the invitation is for lasts, to give the proposed time the same length.
fn length(when: &When) -> TimeDelta {
    match when {
        When::Timed {
            start,
            end: Some(end),
            ..
        } => *end - *start,
        When::Floating {
            start,
            end: Some(end),
            ..
        } => *end - *start,
        When::AllDay { first, last } => TimeDelta::days((*last - *first).num_days() + 1),
        When::Timed { end: None, .. } | When::Floating { end: None, .. } | When::Unstated => {
            AN_HOUR
        }
    }
}

/// Propose to the organiser of the invitation in `message` that it start at `start` instead,
/// lasting as long as it does now, with a `comment` saying why when there is one.
///
/// Only an invitation to the reader that asks for an answer can be countered: not a
/// cancellation, an answer, an event the reader organised, or one that names no organiser.
pub fn propose(
    store: &SqliteStore,
    message: MessageId,
    start: DateTime<Utc>,
    comment: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Proposed, CoreError> {
    let Asked {
        original,
        bytes,
        calendar,
        invite,
        address,
        organiser,
        identity,
    } = asked(store, message)?;
    let event = calendar.main_event().ok_or(CoreError::NoInvitation)?;
    let end = start + length(&invite.when);
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    let calendar_text = ical::counter(
        event,
        &Proposing {
            attendee: &address,
            start,
            end,
            comment,
            at: now,
            product: &product(),
        },
    )?;

    let title = invite.title.as_deref().unwrap_or("(no title)");
    let subject = format!("New time proposed: {}", one_line(title));
    let proposed = When::Timed {
        start,
        end: Some(end),
        zone: ical::EventZone::Utc,
    };
    // Written for the organiser in UTC, as an answer's is: their clock is not known here.
    let said = show_when(&proposed, &Utc, crate::when::Clock::H24);
    let mut text = format!(
        "{} proposes a new time for \"{}\".\r\n\r\nProposed: {}\r\n",
        identity.from.shown(),
        one_line(title),
        said.yours
    );
    if let Some(comment) = comment {
        text.push_str(&format!("\r\n{}\r\n", comment.replace('\n', "\r\n")));
    }
    let sent = Itip {
        method: "COUNTER",
        subject: &subject,
        text: &text,
        calendar: &calendar_text,
    };
    let to = queue(
        store,
        &original,
        &bytes,
        &identity.from,
        &organiser,
        &sent,
        now,
    )?;
    Ok(Proposed { to })
}
