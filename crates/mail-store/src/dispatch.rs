//! Addressing a queued operation at the moment it is sent, shared by both stores.
//!
//! A queued operation names local messages, and the server addresses it is sent to are looked
//! up when it leaves rather than when it was queued. Looked up at queue time, a second move of a
//! message queued behind a first named the address the first moved it away from: `UID MOVE` of
//! a UID no longer in the mailbox is not an error (RFC 9051), so the second move answered OK,
//! settled, and was lost (FINDINGS F153).

use crate::{Dispatch, StoreError};
use mail_domain::{MessageId, ProtoOp, RemoteIntent, RemoteRef};
use std::collections::{HashMap, HashSet};

/// Where one message is on the server now: `None` when this client no longer holds it, and an
/// empty list when it holds it at no address — moved by a server that did not say where to.
pub(crate) type Lookup<'a> = &'a dyn Fn(MessageId) -> Result<Option<Vec<RemoteRef>>, StoreError>;

/// The messages an intent is about: none for one that names no message already held.
fn messages_of(intent: &RemoteIntent) -> &[MessageId] {
    match intent {
        RemoteIntent::SetFlags { messages, .. }
        | RemoteIntent::SetMailbox { messages, .. }
        | RemoteIntent::SetLabels { messages, .. }
        | RemoteIntent::File { messages, .. }
        | RemoteIntent::AddKeyword { messages, .. } => messages,
        RemoteIntent::Send { .. } | RemoteIntent::Folder(_) | RemoteIntent::Append { .. } => &[],
    }
}

/// The messages of `intent` the server holds, each once: what a queued entry keeps, to be
/// addressed again when it is sent. `on_server` says whether one has a server address now, or
/// is waiting for a sync to find where a move put it. A message that is neither is left out for
/// good, as it always was: it is mail the server has never held.
pub(crate) fn addressed(
    intent: &RemoteIntent,
    on_server: &dyn Fn(MessageId) -> Result<bool, StoreError>,
) -> Result<Vec<MessageId>, StoreError> {
    let mut out: Vec<MessageId> = Vec::new();
    for message in messages_of(intent) {
        if !out.contains(message) && on_server(*message)? {
            out.push(*message);
        }
    }
    Ok(out)
}

/// `op` pointed at `remotes`, when it is an operation on messages; anything else as it is.
pub(crate) fn readdress(op: ProtoOp, remotes: Vec<RemoteRef>) -> ProtoOp {
    match op {
        ProtoOp::SetFlags { read, star, .. } => ProtoOp::SetFlags {
            remotes,
            read,
            star,
        },
        ProtoOp::SetMailbox { role, .. } => ProtoOp::SetMailbox { remotes, role },
        ProtoOp::SetLabels { add, remove, .. } => ProtoOp::SetLabels {
            remotes,
            add,
            remove,
        },
        ProtoOp::File { folder, .. } => ProtoOp::File { remotes, folder },
        ProtoOp::AddKeyword { keyword, .. } => ProtoOp::AddKeyword { remotes, keyword },
        other => other,
    }
}

/// What one entry, about `messages`, asks of the server now, on its own account.
///
/// No messages means an operation that names none — a send, an upload, folder work, or a row
/// queued before the outbox kept its messages — and is sent as it was queued. A message no
/// longer held is left out; when that is every one, there is nothing to say. A held message
/// with no address makes the whole entry wait: sending it to the rest would settle it, and the
/// change would never reach the one that is waiting.
pub(crate) fn own(
    op: ProtoOp,
    messages: &[MessageId],
    addresses: Lookup<'_>,
) -> Result<Dispatch, StoreError> {
    if messages.is_empty() {
        return Ok(Dispatch::Send(op));
    }
    let mut remotes: Vec<RemoteRef> = Vec::new();
    let mut held = false;
    for message in messages {
        match addresses(*message)? {
            None => {}
            Some(found) if found.is_empty() => return Ok(Dispatch::Wait),
            Some(found) => {
                held = true;
                for remote in found {
                    if !remotes.contains(&remote) {
                        remotes.push(remote);
                    }
                }
            }
        }
    }
    if !held {
        return Ok(Dispatch::Moot);
    }
    Ok(Dispatch::Send(readdress(op, remotes)))
}

/// What an entry asks of the server now, given the messages of every entry queued before it on
/// its account, in queue order.
///
/// As [`own`], and it also waits behind an earlier entry that waits and shares a message with it, however that
/// one came to wait: two operations on one message reach the server in the order they were
/// made, or the earlier one, sent later, undoes the later one. Only the messages of the entries
/// before it are read, never their operations.
pub(crate) fn in_queue(
    earlier: &[Vec<MessageId>],
    op: ProtoOp,
    messages: &[MessageId],
    addresses: Lookup<'_>,
) -> Result<Dispatch, StoreError> {
    let known: HashMap<MessageId, Option<Vec<RemoteRef>>> = earlier
        .iter()
        .flatten()
        .chain(messages)
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|m| addresses(m).map(|found| (m, found)))
        .collect::<Result<_, _>>()?;
    let cached = |m: MessageId| Ok(known.get(&m).cloned().flatten());
    let mut blocked: HashSet<MessageId> = HashSet::new();
    for before in earlier {
        let waits = before.iter().any(|m| blocked.contains(m))
            || before
                .iter()
                .any(|m| matches!(known.get(m), Some(Some(found)) if found.is_empty()));
        if waits {
            blocked.extend(before.iter().copied());
        }
    }
    if messages.iter().any(|m| blocked.contains(m)) {
        return Ok(Dispatch::Wait);
    }
    own(op, messages, &cached)
}
