//! Putting a conversation off until later, and bringing it back.
//!
//! `Op::SetSnooze` has been in the domain since phase 1 and `Filter::Snoozed`/`SnoozeDue` have
//! been answerable by the store for as long — and nothing could set one. `op_for` returns `None`
//! for `OpKind::Snooze` because the op needs a payload, and no surface supplied one, so the
//! whole apparatus sat there unused.
//!
//! Nothing runs when a snooze expires and nothing needs to. `SnoozeDue` is resolved against
//! `now` at query time, so a conversation returns to the inbox on the stroke whether the client
//! was running or not — which is the right design for something that may be asleep for a week.

use crate::view::snooze_until;
use chrono::{DateTime, Local, Utc};
use mail_domain::{
    AccountCaps, ArchiveMeans, ChangeId, Condstore, ExpungeMeans, FolderRoles, Message, MoveExt,
    Op, ServerLabels, ServerThreads, Snooze, Supported, Target, ThreadId, WatchMode,
};
use mail_store::{SqliteStore, Store};

/// Put a conversation off until `phrase` names.
pub fn snooze(
    store: &SqliteStore,
    thread: ThreadId,
    phrase: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let at = snooze_until(phrase, now, &Local)?;
    set(store, thread, Snooze::Until(at), now)?;
    Ok(format!(
        "snoozed until {}\n",
        crate::view::stamp(at, &Local, crate::view::Stamp::Full)
    ))
}

/// Bring one back now.
pub fn wake(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) -> Result<String, String> {
    set(store, thread, Snooze::Inactive, now)?;
    Ok("back in the inbox\n".to_owned())
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
) -> Result<(), String> {
    let loaded = store.thread(thread).map_err(|e| e.to_string())?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let account = messages
        .first()
        .map(|m| m.account)
        .ok_or_else(|| "that conversation has no messages".to_owned())?;

    let applied = Op::SetSnooze(snooze).apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &local_only(now),
        now,
    );
    store
        .apply(account, &applied.forward)
        .map_err(|e| e.to_string())?;
    let _ = ChangeId::generate();
    Ok(())
}

/// Capabilities for an operation that never leaves this machine.
///
/// Snooze is local by construction: no protocol represents it, so what a server supports cannot
/// change the answer. Spelled out rather than read from the account because reading them would
/// suggest they matter.
fn local_only(now: DateTime<Utc>) -> AccountCaps {
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
