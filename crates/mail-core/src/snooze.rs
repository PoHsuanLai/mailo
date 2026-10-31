//! Putting a conversation off until later, and bringing it back.
//!
//! `op_for` returns `None` for `OpKind::Snooze` because `Op::SetSnooze` needs a payload, the
//! instant to come back at; this module supplies it.
//!
//! Nothing is written when a snooze expires. `SnoozeDue` is resolved against `now` at query time,
//! so a conversation returns to the inbox on the stroke whether the client was running or not;
//! a window that is open asks again at [`next_wake`], so its list shows it come back.

use crate::error::CoreError;
use crate::error::Logged;
use chrono::{DateTime, Local, Utc};
use mail_domain::{
    AccountCaps, ArchiveMeans, Condstore, ExpungeMeans, Filter, FolderRoles, Message, MoveExt, Op,
    PageReq, Property, Query, ServerLabels, ServerThreads, Snooze, Sort, SortDir, Supported,
    Target, ThreadId, WatchMode,
};
use mail_logic::snooze::snooze_until;
use mail_store::{SqliteStore, Store};

/// Put a conversation off until `phrase` names, and say when that is.
pub fn snooze(
    store: &SqliteStore,
    thread: ThreadId,
    phrase: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, CoreError> {
    let at = snooze_until(phrase, now, &Local)?;
    set(store, thread, Snooze::Until(at), now)?;
    Ok(at)
}

/// When the next snoozed conversation comes back: the earliest snooze still ahead of `now`, or
/// `None` with none waiting (or none readable, which is logged).
pub fn next_wake(store: &SqliteStore, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let filter = Filter::And(vec![
        Filter::Snoozed,
        Filter::Not(Box::new(Filter::SnoozeDue)),
    ]);
    let mut next = None;
    let mut after = None;
    loop {
        let query = Query {
            filter: filter.clone(),
            sort: Sort {
                property: Property::Date,
                dir: SortDir::Desc,
            },
            page: PageReq { after, limit: 500 },
        };
        let Some(page) = store
            .threads(&query, now)
            .or_log("the snoozed conversations could not be read")
        else {
            return next;
        };
        next = page
            .items
            .iter()
            .filter_map(|summary| match summary.snooze {
                Snooze::Until(at) if at > now => Some(at),
                _ => None,
            })
            .chain(next)
            .min();
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => return next,
        }
    }
}

/// Bring one back now.
pub fn wake(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) -> Result<(), CoreError> {
    set(store, thread, Snooze::Inactive, now)
}

/// Pin a conversation, or unpin it if it is already pinned. Says whether it is pinned now.
///
/// Here rather than in a module of its own because pin and snooze are the same shape: thread
/// level, local by construction, and needing a payload that `op_for` cannot supply.
pub fn pin(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) -> Result<bool, CoreError> {
    let loaded = store.thread(thread)?;
    let op = mail_logic::mailbox_filter::pin_op(&loaded.summary, now);
    let pinned = matches!(op, Op::SetPin(mail_domain::Pin::Rank(_)));
    apply(store, thread, op, now)?;
    Ok(pinned)
}

/// Apply `Op::SetSnooze` to a thread.
///
/// Through `Op::apply` and `Store::apply` like every other operation, so the change is recorded
/// with its inverse and can be undone. Snooze has no server representation anywhere — it is
/// local by construction, which is why the capabilities below are the safe defaults rather than
/// anything read from an account.
fn set(
    store: &SqliteStore,
    thread: ThreadId,
    snooze: Snooze,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    apply(store, thread, Op::SetSnooze(snooze), now)
}

/// Apply a thread-level op through `Op::apply` and `Store::apply`, like every other operation,
/// so the change is recorded with its inverse and can be undone.
pub(crate) fn apply(
    store: &SqliteStore,
    thread: ThreadId,
    op: Op,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let loaded = store.thread(thread)?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).or_log("a message could not be read"))
        .collect();
    let account = messages
        .first()
        .map(|m| m.account.clone())
        .ok_or(CoreError::EmptyConversation)?;

    let applied = op.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &local_only(now),
        now,
    );
    store.apply(account, &applied.forward)?;
    Ok(())
}

/// Capabilities for an operation that never leaves this machine.
///
/// Snooze is local by construction: no protocol represents it, so what a server supports cannot
/// change the answer. Spelled out rather than read from the account because reading them would
/// suggest they matter.
pub(crate) fn local_only(now: DateTime<Utc>) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: Default::default(),
        observed_at: now,
    }
}
