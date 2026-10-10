//! Remind me if no reply.
//!
//! A conversation the user has written to can carry a reminder ([`FollowUp`]): if nobody else
//! has written by a time the user chose, the conversation comes back to the top of the inbox,
//! marked "No reply yet", and a notification says so. If someone did write, the reminder goes
//! quietly.
//!
//! Nothing here runs on a timer of its own. [`sweep`] is a pure-enough pass over the store — the
//! reminders, their conversations' messages and the user's own addresses — and whoever is
//! running calls it: the window at launch and again when the next reminder comes due, and
//! `mailo watch` after each pass. A reminder that came due while mailo was closed is caught up
//! by the first sweep after it opens, and because coming back is a change of state
//! ([`FollowUp::Returned`]) made once, it is announced once, by whichever sweep made it.
//!
//! "A reply" is any message in the conversation from an address that is not one of the user's
//! own, dated after the reminder was set. Its place does not matter: an answer filed by a rule
//! is still an answer.
//!
//! A reminder asked for in the composer waits in `follow_up_held` (migration 0027) until its
//! message has left: a reply joins its conversation once the outbox has sent it, a new message
//! the conversation its copy lands in when the Sent folder is next synced, or at once on an
//! account with no Sent folder (POP3), whose copy is kept here as it is sent. One whose copy is
//! never found is let go a week after it was due.

use crate::error::{CoreError, Logged, TimeError};
use crate::notify::{Notification, Notifier, Opens, Own};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::{
    Filter, FollowUp, MailboxRole, MatchCtx, Message, Op, Snooze, ThreadId, ThreadSummary,
};
use mail_store::{SqliteStore, Store};

use held::landed;
pub use held::{Held, after_queue, held, hold, release};

/// The times a reminder offers by name, as `(key, what it says)`. A typed time is the rest.
pub const CHOICES: [(&str, &str); 3] = [
    ("tomorrow", "Tomorrow morning"),
    ("three-days", "In 3 days"),
    ("next-week", "Next week"),
];

/// How long after it was due a composer's reminder still waits for its message to be found.
const STALE: TimeDelta = TimeDelta::days(7);

/// The instant a reminder chosen as `choice` comes due, counted from `from` in `zone`: one of
/// [`CHOICES`]' keys, or a time typed the way snooze reads one (`fri 17:00`, `+2h`,
/// `2026-10-02`). A time that is not after `from` is refused, in words: a reminder for a moment
/// already gone would come back before anyone could answer.
pub fn due<Tz: TimeZone>(
    choice: &str,
    from: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, TimeError>
where
    Tz::Offset: std::fmt::Display,
{
    let phrase = match choice {
        "tomorrow" => "tomorrow".to_owned(),
        // Three mornings on: the day someone can be expected to have answered by.
        "three-days" => {
            let day = from.with_timezone(zone).date_naive() + TimeDelta::days(3);
            day.format("%Y-%m-%d").to_string()
        }
        "next-week" => "monday".to_owned(),
        typed => typed.trim().to_owned(),
    };
    if phrase.is_empty() {
        return Err(TimeError::Blank);
    }
    let at = crate::snooze::snooze_until(&phrase, from, zone)?;
    if at <= from {
        return Err(TimeError::Passed {
            stamp: crate::when::stamp(at, zone, crate::when::Stamp::Full),
        });
    }
    Ok(at)
}

/// Whether anyone other than the user has written in `messages` since `set`.
pub fn replied(messages: &[Message], own: &Own, set: DateTime<Utc>) -> bool {
    messages
        .iter()
        .any(|message| message.date > set && !own.includes(&message.from.email))
}

/// Whether a conversation belongs where the list is narrowed to.
pub(crate) fn in_scope(summary: &ThreadSummary, scope: Option<&Filter>, now: DateTime<Utc>) -> bool {
    scope.is_none_or(|filter| {
        filter.fit(&MatchCtx {
            summary,
            corpus: None,
            folders: &[],
            now,
        })
    })
}

/// The conversations whose reminder came back with no reply, most recently returned first, for
/// the top of the inbox.
///
/// Only while the conversation is still somewhere a person looks: in the inbox or in Sent, and
/// not snoozed away. Archiving it, trashing it or snoozing it is the user's answer to the
/// reminder, so the row leaves the way any other row would.
pub fn returned(
    store: &SqliteStore,
    scope: Option<&Filter>,
    now: DateTime<Utc>,
) -> Vec<ThreadSummary> {
    let mut back: Vec<ThreadSummary> = store
        .follow_ups()
        .or_log_default("the reminders could not be read")
        .into_iter()
        .filter(|summary| matches!(summary.follow_up, FollowUp::Returned { .. }))
        .filter(|summary| {
            summary.mailboxes.contains(MailboxRole::Inbox)
                || summary.mailboxes.contains(MailboxRole::Sent)
        })
        .filter(|summary| !matches!(summary.snooze, Snooze::Until(at) if at > now))
        .filter(|summary| in_scope(summary, scope, now))
        .collect();
    // The store lists them soonest due first; the one that came back last goes on top.
    back.reverse();
    back
}

/// Every conversation with a reminder, soonest due first: the Waiting place.
pub fn waiting(
    store: &SqliteStore,
    scope: Option<&Filter>,
    now: DateTime<Utc>,
) -> Vec<ThreadSummary> {
    store
        .follow_ups()
        .or_log_default("the reminders could not be read")
        .into_iter()
        .filter(|summary| in_scope(summary, scope, now))
        .collect()
}

/// `top` above `rows`, each conversation once.
pub fn on_top(top: Vec<ThreadSummary>, rows: Vec<ThreadSummary>) -> Vec<ThreadSummary> {
    let ids: Vec<ThreadId> = top.iter().map(|summary| summary.id).collect();
    top.into_iter()
        .chain(
            rows.into_iter()
                .filter(|summary| !ids.contains(&summary.id)),
        )
        .collect()
}

/// What a row says about its reminder, when it says anything: the one words a returned
/// conversation shows, beside nothing for a reminder still waiting.
pub fn row_words(follow_up: &FollowUp) -> Option<&'static str> {
    match follow_up {
        FollowUp::Returned { .. } => Some("No reply yet"),
        FollowUp::Until { .. } | FollowUp::Inactive => None,
    }
}

/// The op a menu applies to set a reminder at `at`, asked for at `set`.
pub fn remind(at: DateTime<Utc>, set: DateTime<Utc>) -> Op {
    Op::SetFollowUp(FollowUp::Until { at, set })
}

/// What one [`sweep`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Swept {
    /// Conversations whose reminder came due with no reply, as they now are.
    pub returned: Vec<ThreadSummary>,
    /// Conversations whose reminder went because someone replied.
    pub cleared: Vec<ThreadId>,
    /// Conversations a composer's reminder joined.
    pub attached: Vec<ThreadId>,
}

impl Swept {
    /// Whether the sweep changed anything the window shows.
    pub fn changed(&self) -> bool {
        !(self.returned.is_empty() && self.cleared.is_empty() && self.attached.is_empty())
    }
}

/// Move every reminder on to where `now` puts it: a held one onto its conversation once its
/// message has left, a reply's arrival clears one, and one due with no reply comes back.
///
/// Each change goes through [`Op::apply`] and [`Store::apply`] like the user's own, local by
/// construction, and none is put on anyone's undo stack: the user did not do them.
pub fn sweep(store: &SqliteStore, now: DateTime<Utc>) -> Result<Swept, CoreError> {
    let mut swept = Swept::default();
    for held in held(store) {
        match landed(store, &held) {
            Some(thread) => {
                crate::snooze::apply(store, thread, remind(held.at, held.set), now)?;
                release(store, held.draft)?;
                swept.attached.push(thread);
            }
            None if held.at + STALE <= now => release(store, held.draft)?,
            None => {}
        }
    }
    let own = crate::notify::own_addresses(store)?;
    for summary in store.follow_ups()? {
        let set = match summary.follow_up {
            FollowUp::Until { set, .. } | FollowUp::Returned { set, .. } => set,
            FollowUp::Inactive => continue,
        };
        let loaded = store.thread(summary.id)?;
        let messages: Vec<Message> = loaded
            .messages
            .iter()
            .filter_map(|id| store.message(*id).ok())
            .collect();
        let next = match summary.follow_up {
            _ if replied(&messages, &own, set) => FollowUp::Inactive,
            FollowUp::Until { at, set } if at <= now => FollowUp::Returned { at, set },
            other => other,
        };
        if next == summary.follow_up {
            continue;
        }
        crate::snooze::apply(store, summary.id, Op::SetFollowUp(next), now)?;
        match next {
            FollowUp::Inactive => swept.cleared.push(summary.id),
            FollowUp::Returned { .. } => swept.returned.push(ThreadSummary {
                follow_up: next,
                ..summary
            }),
            FollowUp::Until { .. } => {}
        }
    }
    Ok(swept)
}

/// When the next sweep has something to do on its own: the soonest reminder still waiting.
/// `None` when nothing waits.
///
/// A held reminder is not counted. It joins its conversation when its message is found, which a
/// sync or a send brings about, and whoever runs the sweep sweeps again after those.
pub fn next_due(store: &SqliteStore) -> Option<DateTime<Utc>> {
    store
        .follow_ups()
        .or_log_default("the reminders could not be read")
        .into_iter()
        .filter_map(|summary| match summary.follow_up {
            FollowUp::Until { at, .. } => Some(at),
            FollowUp::Returned { .. } | FollowUp::Inactive => None,
        })
        .min()
}

/// The notifications a sweep's returned conversations raise: one each, naming the conversation.
pub fn notifications(returned: &[ThreadSummary]) -> Vec<Notification> {
    returned
        .iter()
        .map(|summary| Notification {
            account: summary.account.clone(),
            summary: "No reply yet".to_owned(),
            body: if summary.subject.trim().is_empty() {
                "(no subject)".to_owned()
            } else {
                summary.subject.clone()
            },
            opens: Opens::Thread(summary.id),
        })
        .collect()
}

/// Sweep, and say what came back through `notifier`. Returns what the sweep did.
pub fn sweep_and_announce(
    store: &SqliteStore,
    notifier: Option<&dyn Notifier>,
    now: DateTime<Utc>,
) -> Result<Swept, CoreError> {
    let swept = sweep(store, now)?;
    if let Some(notifier) = notifier {
        for notification in notifications(&swept.returned) {
            notifier.show(&notification);
        }
    }
    Ok(swept)
}

mod held;
#[cfg(test)]
mod tests;
