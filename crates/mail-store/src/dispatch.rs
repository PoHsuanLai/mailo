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

/// Where one message is on the server now.
pub(crate) type Lookup<'a> = &'a dyn Fn(MessageId) -> Result<Place, StoreError>;

/// Complete syncs of the folder a move put a message in that may miss it before an operation
/// waiting on it is given up ([`Dispatch::Lost`], FINDINGS F155).
///
/// One ought to be enough: a server answers `MOVE` only once the message is in its destination,
/// under a UID at or above the `UIDNEXT` the next `SELECT` reports, and the next sync of that
/// folder fetches every UID it does not hold. Three leaves room for a server whose front ends see
/// a mailbox a moment late, and for a sync that stopped short without saying so, while a message
/// deleted elsewhere is given up within a few minutes of the app running rather than never.
pub const SYNCS_TO_FIND: u32 = 3;

/// Passes of the account that did not sync that folder in full, after which the wait is given up
/// all the same.
///
/// These are not syncs that should have found it, and a folder that fails a pass or two is not a
/// reason to give up on it. But a folder this client does not sync at all — one the user does not
/// follow, or `Spam` — is never synced, and without this the wait would again be for good. A pass
/// is a minute to five while the app runs, so this is somewhere between twenty minutes and an hour
/// and a half of running, and no time at all while it does not.
pub const PASSES_TO_FIND: u32 = 20;

/// Where a message an operation names is, as the moment it is sent sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    /// No longer held by this client.
    Gone,
    /// Held, at these addresses. None means held at no address a sync is looking for.
    At(Vec<RemoteRef>),
    /// Held at no address: the server moved it without saying where, and a sync is to find it.
    Unplaced(Unplaced),
}

/// A message the server moved without saying where, and how long it has been looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unplaced {
    /// The folder the move filed it into, where that was kept.
    pub(crate) mailbox: Option<String>,
    /// Complete syncs of that folder since, none of which found it.
    pub(crate) syncs: u32,
    /// Passes since that did not sync that folder in full.
    pub(crate) passes: u32,
}

impl Unplaced {
    /// Why an operation on this message is given up, or `None` while it is still worth waiting.
    pub(crate) fn given_up(&self) -> Option<String> {
        let told = "The server moved this message without saying where it put it";
        let undone = "so this change was not sent and has been undone here";
        match &self.mailbox {
            Some(folder) if self.syncs >= SYNCS_TO_FIND => Some(format!(
                "{told}, and {} syncs of {folder} since have not found it there, {undone}.",
                self.syncs
            )),
            Some(folder) if self.passes >= PASSES_TO_FIND => Some(format!(
                "{told}, into {folder}, and {folder} has not been synced in the {} passes \
                 since, {undone}.",
                self.passes
            )),
            None if self.passes >= PASSES_TO_FIND => Some(format!(
                "{told}, and {} passes since have not found it, {undone}.",
                self.passes
            )),
            _ => None,
        }
    }

    /// This row after one pass of its account that synced `synced` in full: a sync of its folder
    /// that did not find it, or a pass that did not look.
    pub(crate) fn after_pass(&mut self, synced: &[String]) {
        let looked = self
            .mailbox
            .as_deref()
            .is_some_and(|folder| synced.iter().any(|s| same_folder(s, folder)));
        if looked {
            self.syncs += 1;
        } else {
            self.passes += 1;
        }
    }
}

/// Whether two paths name one folder: exactly, or `INBOX` in any case (RFC 9051 §5.1).
pub(crate) fn same_folder(a: &str, b: &str) -> bool {
    a == b || (a.eq_ignore_ascii_case("INBOX") && b.eq_ignore_ascii_case("INBOX"))
}

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
/// change would never reach the one that is waiting. Once the syncs that should have found that
/// message have not, the whole entry is given up, for the same reason ([`Unplaced::given_up`]).
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
    let mut waits = false;
    for message in messages {
        match addresses(*message)? {
            Place::Gone => {}
            Place::Unplaced(unplaced) => match unplaced.given_up() {
                Some(why) => return Ok(Dispatch::Lost(why)),
                None => waits = true,
            },
            Place::At(found) if found.is_empty() => waits = true,
            Place::At(found) => {
                held = true;
                for remote in found {
                    if !remotes.contains(&remote) {
                        remotes.push(remote);
                    }
                }
            }
        }
    }
    if waits {
        return Ok(Dispatch::Wait);
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
/// before it are read, never their operations. An entry naming a message that has been given up
/// waits for nothing: it is refused at its turn, whatever is ahead of it, and holds nothing back.
pub(crate) fn in_queue(
    earlier: &[Vec<MessageId>],
    op: ProtoOp,
    messages: &[MessageId],
    addresses: Lookup<'_>,
) -> Result<Dispatch, StoreError> {
    let known: HashMap<MessageId, Place> = earlier
        .iter()
        .flatten()
        .chain(messages)
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|m| addresses(m).map(|found| (m, found)))
        .collect::<Result<_, _>>()?;
    let cached = |m: MessageId| Ok(known.get(&m).cloned().unwrap_or(Place::Gone));
    let waiting = |m: &MessageId| match known.get(m) {
        Some(Place::At(found)) => found.is_empty(),
        Some(Place::Unplaced(unplaced)) => unplaced.given_up().is_none(),
        Some(Place::Gone) | None => false,
    };
    let lost = |m: &MessageId| matches!(known.get(m), Some(Place::Unplaced(unplaced)) if unplaced.given_up().is_some());
    // It can never be sent, so waiting behind anything would only put off saying so.
    if messages.iter().any(lost) {
        return own(op, messages, &cached);
    }
    let mut blocked: HashSet<MessageId> = HashSet::new();
    for before in earlier {
        let waits = !before.iter().any(lost)
            && (before.iter().any(|m| blocked.contains(m)) || before.iter().any(waiting));
        if waits {
            blocked.extend(before.iter().copied());
        }
    }
    if messages.iter().any(|m| blocked.contains(m)) {
        return Ok(Dispatch::Wait);
    }
    own(op, messages, &cached)
}
