//! What happens to a conversation when it is archived, starred, labelled, snoozed or moved, and
//! how that is taken back.
//!
//! One operation is applied locally and, when the account has a server, queued for it; what it
//! returns is the [`Undo`] that reverses both. The window (`mail_app::ui`) and the intents
//! provider (`mail_app::intents`) are two front-ends over this, so a conversation archived by
//! either is archived the same way: the same patch, the same queued work for the server, the
//! same undo.

use crate::undo::Undo;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// Take back one operation: its inverse, written like any other patch, and the reverse queued
/// for the server when the operation had told it anything.
pub fn take_back(store: &SqliteStore, entry: &Undo) -> bool {
    if store.apply(entry.account, &entry.inverse).is_err() {
        return false;
    }
    withdraw_filing(store, entry);
    if let Some(reverse) = entry
        .remote
        .as_ref()
        .filter(|_| has_server(store, entry.account))
        .and_then(|remote| crate::undo::reverse_intent(remote, &entry.inverse))
    {
        let _ = store.enqueue(entry.account, reverse, &entry.forward, chrono::Utc::now());
    }
    true
}

/// A move to a folder taken back before the server heard of it is taken out of the outbox too.
///
/// A filing has no reverse the server can be sent (`undo::reverse_intent`), so once it has gone
/// the server keeps it filed. Until it has been tried, though, it is only a queued intention, and
/// leaving it queued would move the message on the next sync after the user said not to — and
/// hold its row out of the folder it is back in meanwhile, because a queued move is a message
/// leaving. Settled as refused, which drops the entry and puts back the same values the undo
/// just wrote.
///
/// Not when anything queued after it acts on the same messages. Settling it refused writes its
/// undo, the state from before it, and drops its pending changes, and a later operation on those
/// messages was made on top of it: that operation's own undo and pending changes assume the move
/// is there. Rather than work out which of them survive it, the move is left queued and the
/// server keeps it filed, which is what happened before this existed.
///
/// Nothing marks an entry as taken by a pass that is running, so there is a window this does
/// not close: a sync already sending the move, the window's own or `mailo watch`'s, still
/// reaches the server, and its confirmation then settles an entry that is no longer here. The
/// message is back here and filed there, and the next sync shows it filed.
fn withdraw_filing(store: &SqliteStore, entry: &Undo) {
    let Some(RemoteIntent::File { messages, .. }) = &entry.remote else {
        return;
    };
    // The whole queue, not only what is due: a later operation waiting out a retry or a
    // credential is later all the same.
    let Ok(queued) = store.outbox_due(entry.account, queue_horizon()) else {
        return;
    };
    let Some(filing) = queued.iter().find(|waiting| {
        matches!(waiting.op, ProtoOp::File { .. })
            && waiting.attempts == 0
            && waiting.undo == entry.inverse
    }) else {
        return;
    };
    let mut ours = messages.clone();
    ours.extend(messages_in(&filing.undo));
    if queued
        .iter()
        .filter(|later| later.id > filing.id)
        .any(|later| touches(later, &ours, &filing.op))
    {
        return;
    }
    let _ = store.outbox_settle(
        filing.id,
        mail_store::Settle::Failed {
            reason: "taken back before it was sent".to_owned(),
            retry: Retry::Fatal("taken back".to_owned()),
        },
        chrono::Utc::now(),
    );
}

/// A moment after every queued entry's next attempt, so that `outbox_due` lists the whole queue.
///
/// Year 9999 rather than `DateTime::MAX_UTC`: the store compares times as fixed-width text, and
/// a signed six-digit year sorts before every real one.
fn queue_horizon() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(253_402_300_799, 0).unwrap_or_else(chrono::Utc::now)
}

/// The messages a patch writes to.
fn messages_in(patch: &Patch) -> Vec<MessageId> {
    patch
        .changes
        .iter()
        .filter_map(|change| match change {
            Change::MessageRead(m, _)
            | Change::MessageStar(m, _)
            | Change::MessageMailbox(m, _)
            | Change::MessageLabel(m, _, _)
            | Change::MessageDelete(m) => Some(*m),
            Change::MessageUpsert(message) => Some(message.id),
            _ => None,
        })
        .collect()
}

/// Whether a queued entry acts on any of `messages`, or on any server message `filing` names.
///
/// Its undo names the messages it changed here, and its operation the ones it will change there;
/// either is enough. An operation this cannot read counts as touching them, because guessing
/// wrong that way costs only a move the server is still sent.
fn touches(later: &mail_store::OutboxEntry, messages: &[MessageId], filing: &ProtoOp) -> bool {
    if messages_in(&later.undo)
        .iter()
        .any(|m| messages.contains(m))
    {
        return true;
    }
    let ProtoOp::File { remotes: ours, .. } = filing else {
        return true;
    };
    match &later.op {
        ProtoOp::SetFlags { remotes, .. }
        | ProtoOp::SetMailbox { remotes, .. }
        | ProtoOp::SetLabels { remotes, .. }
        | ProtoOp::File { remotes, .. }
        | ProtoOp::AddKeyword { remotes, .. }
        | ProtoOp::Expunge { remotes }
        | ProtoOp::Destroy { remotes } => remotes.iter().any(|r| ours.contains(r)),
        // A new message, a sent one, or a mailbox: none of them is a message already held.
        ProtoOp::Append { .. } | ProtoOp::Submit { .. } | ProtoOp::Folder(_) => false,
        _ => true,
    }
}

/// Whether `account` has a server to tell: every account but local folders.
fn has_server(store: &SqliteStore, account: AccountId) -> bool {
    !crate::sync::local_accounts(store).contains(&account)
}

/// What the server was last seen to support, or, before the first sync, nothing: a change is
/// then made here alone and the server hears nothing, rather than hearing a guess.
pub fn caps_here(
    store: &SqliteStore,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
) -> AccountCaps {
    crate::sync::caps_of(store, account).unwrap_or(AccountCaps {
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
        connections: ConnectionBudget::default(),
        observed_at: now,
    })
}

/// Apply one resolved operation: locally, and to the server when it has a server half.
///
/// Returns what it takes to undo it, which the window keeps for the toast and ⌘Z.
pub fn perform(store: &SqliteStore, thread: ThreadId, op: Op) -> Option<Undo> {
    let loaded = store.thread(thread).ok()?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let account = messages.first().map(|m| m.account)?;
    // What the server actually supports, which is the only thing that decides whether an
    // operation gets a `RemoteIntent` at all — `Op::remote_intent` is the sole consumer of
    // `caps`, and under `ArchiveMeans::LocalOnly` it returns `None` for Archive and Trash.
    // This used to be a hardcoded struct of safe defaults, so archiving in the window changed
    // nothing on the server and the conversation came back on the next full sync. See F139.
    let caps = caps_here(store, account, chrono::Utc::now());
    let applied = op.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &caps,
        chrono::Utc::now(),
    );
    store.apply(account, &applied.forward).ok()?;
    // And the server's half. `Applied.remote` was computed and dropped on the floor, so every
    // operation the window performed was local and stayed local: a conversation archived here
    // was still in the inbox on the phone, and mail read here was still bold everywhere else.
    // The undo goes with it, because a submission that fails has to put back what it changed.
    //
    // A failure to enqueue is not a failure of the operation: the local change is real and the
    // user can see it. It surfaces where every other stalled submission does, in the outbox.
    //
    // Local folders have no server half: their outbox is never drained, so nothing is put in it.
    if let Some(intent) = applied
        .remote
        .clone()
        .filter(|_| has_server(store, account))
    {
        let _ = store.enqueue(account, intent, &applied.inverse, chrono::Utc::now());
    }
    Some(Undo {
        said: crate::undo::said(&op, &chrono::Local),
        thread: Some(thread),
        account,
        forward: applied.forward,
        inverse: applied.inverse,
        remote: applied.remote,
    })
}

/// Delete forever what `thread` holds in Trash or Spam, here and on the server. Returns the
/// messages it removed; `None` when there were none.
///
/// Not [`perform`]: that applies the patch first and queues after, and removing a message takes
/// its server addresses with it. The deletion is queued first, while the store still knows where
/// each message is (`Store::enqueue`), and nothing is returned to undo it.
pub fn destroy(store: &SqliteStore, thread: ThreadId) -> Option<Vec<MessageId>> {
    let loaded = store.thread(thread).ok()?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let account = messages.first().map(|m| m.account)?;
    let now = chrono::Utc::now();
    let applied = Op::Destroy.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &caps_here(store, account, now),
        now,
    );
    let gone: Vec<MessageId> = applied
        .forward
        .changes
        .iter()
        .filter_map(|change| match change {
            Change::MessageDelete(m) => Some(*m),
            _ => None,
        })
        .collect();
    if gone.is_empty() {
        return None;
    }
    if let Some(intent) = applied.remote.filter(|_| has_server(store, account))
        && store
            .enqueue(account, intent, &applied.inverse, now)
            .is_err()
    {
        // Not removed here either: a message gone here and left there comes back with the next
        // sync, and meanwhile says it was deleted.
        return None;
    }
    store.apply(account, &applied.forward).ok()?;
    Some(gone)
}
