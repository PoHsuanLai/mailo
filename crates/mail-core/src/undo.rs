//! What can be taken back, and the stack that holds it.
//!
//! An entry is an [`Applied`]'s undo kept beside the patch it undoes. Undoing writes the
//! inverse through the store like any other patch, and — when the forward op told the server —
//! queues the reverse for the server too, so an archive undone here is not still archived on
//! the phone.
//!
//! [`Applied`]: mail_domain::Applied

use chrono::{DateTime, Utc};
use mail_domain::{
    Change, DraftId, FolderWork, Hue, LabelId, MailboxRole, Membership, MessageId, NonEmpty, Patch,
    ReadState, RemoteIntent, Star, Subscription, ThreadId,
};
use porter_core::AccountId;
use std::collections::BTreeMap;

/// How many operations the window remembers.
pub const DEPTH: usize = 20;

/// One operation that can be taken back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undo {
    /// The conversation it acted on. `None` for work on an account's folders, which belongs
    /// to no conversation.
    pub thread: Option<ThreadId>,
    pub account: AccountId,
    /// The patch that was applied.
    pub forward: Patch,
    /// The patch that puts it back.
    pub inverse: Patch,
    /// What the forward op queued for the server, if anything.
    pub remote: Option<RemoteIntent>,
    /// A scheduled send the op took back, to be queued again for its time when this is undone.
    ///
    /// Not a patch: what the outbox holds is the message sealed as it was sent, and only the
    /// window, which holds the keys to seal it again, can queue it. [`crate::act::take_back`]
    /// refuses an entry that has one.
    pub resend: Option<Resend>,
}

/// A send held for a time, taken back: the draft, when it was to leave, and how large its pictures
/// were to be sent, which the draft itself does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resend {
    pub draft: DraftId,
    pub at: DateTime<Utc>,
    pub pictures: crate::compose::PictureSize,
}

/// Which entry of the stack an undo means: what a caller carries back when it asks, so the ask
/// takes back the op it named even when a later op has been pushed since.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UndoHandle(pub u64);

/// The most recent gestures, newest last. One entry is everything one gesture did: a single
/// archive, or an archive of every picked conversation, which one undo takes back whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndoStack {
    entries: Vec<(UndoHandle, Vec<Undo>)>,
    /// The handle the next push is given.
    next: u64,
}

impl UndoStack {
    /// Remember `entry`, forgetting the oldest past [`DEPTH`]. The handle takes this entry
    /// back later through [`UndoStack::take`].
    pub fn push(&mut self, entry: Undo) -> UndoHandle {
        self.remember(vec![entry])
    }

    /// Remember what one gesture did to several conversations as one entry, taken back
    /// together. `None`, and nothing remembered, when it did nothing.
    pub fn push_all(&mut self, entries: Vec<Undo>) -> Option<UndoHandle> {
        (!entries.is_empty()).then(|| self.remember(entries))
    }

    fn remember(&mut self, entries: Vec<Undo>) -> UndoHandle {
        let handle = UndoHandle(self.next);
        self.next += 1;
        self.entries.push((handle, entries));
        if self.entries.len() > DEPTH {
            self.entries.remove(0);
        }
        handle
    }

    /// Take the newest entry: every part of the newest gesture, in the order it was done.
    pub fn pop(&mut self) -> Option<Vec<Undo>> {
        self.entries.pop().map(|(_, entry)| entry)
    }

    /// Take the entry `handle` names, wherever it is. `None` once it has been undone or
    /// forgotten.
    pub fn take(&mut self, handle: UndoHandle) -> Option<Vec<Undo>> {
        let at = self.entries.iter().position(|(held, _)| *held == handle)?;
        Some(self.entries.remove(at).1)
    }

    /// Forget every part that changed one of `messages`, and an entry left with no part.
    ///
    /// For mail deleted forever: taking back an earlier move of it would write to a message
    /// that is no longer here, and say it had done something.
    pub fn forget_messages(&mut self, messages: &[MessageId]) {
        let names = |patch: &Patch| {
            patch.changes.iter().any(|change| match change {
                Change::MessageRead(m, _)
                | Change::MessageStar(m, _)
                | Change::MessageFlag(m, _)
                | Change::MessageMailbox(m, _)
                | Change::MessageLabel(m, _, _)
                | Change::MessageDelete(m) => messages.contains(m),
                Change::MessageUpsert(message) => messages.contains(&message.id),
                Change::ThreadSnooze(..)
                | Change::ThreadPin(..)
                | Change::ThreadMute(..)
                | Change::ThreadFollowUp(..)
                | Change::LabelUpsert(_)
                | Change::DraftUpsert(_)
                | Change::DraftDelete(_)
                | Change::LabelRemove(_)
                | Change::FolderUpsert(_)
                | Change::FolderRemove(_)
                | Change::FolderRename { .. } => false,
            })
        };
        for (_, parts) in &mut self.entries {
            parts.retain(|part| !names(&part.forward) && !names(&part.inverse));
        }
        self.entries.retain(|(_, parts)| !parts.is_empty());
    }

    /// The newest entry, left in place.
    pub fn last(&self) -> Option<&[Undo]> {
        self.entries.last().map(|(_, entry)| entry.as_slice())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What the server must be told to undo `remote`, read from the inverse patch.
///
/// Only the kind of change the forward op sent: an archive that was local-only (no `remote`)
/// has nothing to reverse on the server, and a thread's snooze never had a server half at all.
/// `None` when the inverse changes nothing of that kind.
pub(crate) fn reverse_intent(remote: &RemoteIntent, inverse: &Patch) -> Option<RemoteIntent> {
    match remote {
        RemoteIntent::SetMailbox { .. } => {
            let mut by_role: BTreeMap<MailboxRole, Vec<MessageId>> = BTreeMap::new();
            for change in &inverse.changes {
                if let Change::MessageMailbox(id, role) = change {
                    by_role.entry(*role).or_default().push(*id);
                }
            }
            // One intent carries one role. An undo that scatters messages back to several
            // places is rare (a thread archived from two folders) and goes back by the largest.
            by_role
                .into_iter()
                .max_by_key(|(_, ids)| ids.len())
                .map(|(role, messages)| RemoteIntent::SetMailbox { messages, role })
        }
        RemoteIntent::SetFlag { messages, hue } => reverse_flag(messages, *hue, inverse),
        RemoteIntent::SetFlags { .. } => {
            if let Some(coloured) = recolour(inverse) {
                return Some(coloured);
            }
            let mut messages = Vec::new();
            let mut read: Option<ReadState> = None;
            let mut star: Option<Star> = None;
            for change in &inverse.changes {
                match change {
                    Change::MessageRead(id, state) => {
                        read = Some(*state);
                        messages.push(*id);
                    }
                    Change::MessageStar(id, state) => {
                        star = Some(*state);
                        messages.push(*id);
                    }
                    _ => {}
                }
            }
            messages.dedup();
            (!messages.is_empty()).then_some(RemoteIntent::SetFlags {
                messages,
                read,
                star,
            })
        }
        RemoteIntent::SetLabels { .. } => {
            let mut messages = Vec::new();
            let mut add: Vec<LabelId> = Vec::new();
            let mut remove: Vec<LabelId> = Vec::new();
            for change in &inverse.changes {
                if let Change::MessageLabel(id, label, membership) = change {
                    messages.push(*id);
                    let side = match membership {
                        Membership::In => &mut add,
                        Membership::Out => &mut remove,
                    };
                    if !side.contains(label) {
                        side.push(*label);
                    }
                }
            }
            messages.dedup();
            (!messages.is_empty()).then_some(RemoteIntent::SetLabels {
                messages,
                add,
                remove,
            })
        }
        RemoteIntent::Folder(work) => reverse_folder(work).map(RemoteIntent::Folder),
        // A submission is not taken back by a patch; the outbox owns that. Nor is a keyword:
        // `$MDNSent` records a receipt answered, which no undo un-answers.
        //
        // Nor a filing: the message has left the mailbox this client knew it by, and the server
        // address that would name it in the folder arrives only with that folder's next sync.
        // The undo puts it back here; a filing the server has been sent stays filed there, and
        // one still waiting is taken out of the outbox by the window's undo (`ui::ops`).
        //
        // Nor a copy, for the same reason: the copy's address arrives with that folder's next
        // sync, or with the server's answer. The undo takes it out of the folder here, and one
        // still waiting is taken out of the outbox as a filing is; a copy the server has made
        // is not taken back at all (`crate::act::take_back` refuses it), or the next sync would
        // put it back in the folder.
        //
        // Nor, above all, a deletion forever: the server's copy is gone once it is sent, and
        // the window never offers one an undo (`crate::destroy`).
        //
        // A draft's copy on the server follows the draft, which has undo of its own: a discard
        // taken back puts the draft here, and its next save puts a copy there again.
        RemoteIntent::Send { .. }
        | RemoteIntent::AddKeyword { .. }
        | RemoteIntent::Append { .. }
        | RemoteIntent::File { .. }
        | RemoteIntent::Copy { .. }
        | RemoteIntent::Destroy { .. }
        | RemoteIntent::KeepDraft { .. }
        | RemoteIntent::DropDraft { .. } => None,
    }
}

/// What a flag in `hue` on `messages` was before, said to the server: each message as its own
/// star and colour in `inverse` say, or as flagged in `hue` where they say nothing.
///
/// One intent carries one state. An undo that leaves messages in several (a thread with one red
/// flag flagged orange) goes back by the largest, as an undone archive does.
fn reverse_flag(messages: &[MessageId], hue: Hue, inverse: &Patch) -> Option<RemoteIntent> {
    let mut by_state: BTreeMap<Option<Hue>, Vec<MessageId>> = BTreeMap::new();
    for message in messages {
        let unflagged = inverse.changes.iter().any(
            |change| matches!(change, Change::MessageStar(id, Star::Unstarred) if id == message),
        );
        let was = inverse.changes.iter().find_map(|change| match change {
            Change::MessageFlag(id, before) if id == message => Some(*before),
            _ => None,
        });
        let state = (!unflagged).then_some(was.unwrap_or(hue));
        by_state.entry(state).or_default().push(*message);
    }
    let (state, messages) = by_state.into_iter().max_by_key(|(_, ids)| ids.len())?;
    Some(match state {
        Some(hue) => RemoteIntent::SetFlag { messages, hue },
        None => RemoteIntent::SetFlags {
            messages,
            read: None,
            star: Some(Star::Unstarred),
        },
    })
}

/// An unflagging undone, where a flag had a colour: the server took the colour away with the
/// flag, so the flag goes back in its colour rather than plain. One intent carries one colour,
/// so a mixed set goes back by the colour most of them wore; red is plain `\Flagged` and wins a
/// tie. `None` where red is the most: the plain star restores those, and the colour of the few
/// that wore another is the one thing the undo cannot give back in the same breath.
fn recolour(inverse: &Patch) -> Option<RemoteIntent> {
    let mut wore: BTreeMap<MessageId, Hue> = BTreeMap::new();
    let mut starred: Vec<MessageId> = Vec::new();
    for change in &inverse.changes {
        match change {
            Change::MessageFlag(id, hue) => {
                wore.insert(*id, *hue);
            }
            Change::MessageStar(id, Star::Starred) => starred.push(*id),
            _ => {}
        }
    }
    for id in wore.keys() {
        if !starred.contains(id) {
            starred.push(*id);
        }
    }
    let mut by_hue: BTreeMap<Hue, Vec<MessageId>> = BTreeMap::new();
    for id in starred {
        by_hue
            .entry(wore.get(&id).copied().unwrap_or(Hue::Red))
            .or_default()
            .push(id);
    }
    let (hue, messages) = by_hue
        .into_iter()
        .max_by_key(|(hue, ids)| (ids.len(), *hue == Hue::Red))?;
    (hue != Hue::Red).then_some(RemoteIntent::SetFlag { messages, hue })
}

/// The folder work that takes `work` back on the server.
///
/// Read from the work rather than the inverse patch, because the patch describes this client's
/// records and the server needs the command. A delete that took mail with it has no reverse:
/// making the folder again would bring back a name, not the messages, so that one is not
/// offered as an undo at all.
pub fn reverse_folder(work: &FolderWork) -> Option<FolderWork> {
    match work {
        FolderWork::Create { path } => Some(FolderWork::Delete {
            path: path.clone(),
            // Whatever was filed there since is the server's to refuse over.
            non_empty: NonEmpty::Refuse,
        }),
        FolderWork::Rename { from, to } => Some(FolderWork::Rename {
            from: to.clone(),
            to: from.clone(),
        }),
        FolderWork::Delete {
            path,
            non_empty: NonEmpty::Refuse,
        } => Some(FolderWork::Create { path: path.clone() }),
        FolderWork::Delete {
            non_empty: NonEmpty::Allow,
            ..
        } => None,
        FolderWork::Subscribe { path, subscription } => Some(FolderWork::Subscribe {
            path: path.clone(),
            subscription: match subscription {
                Subscription::Subscribed => Subscription::Unsubscribed,
                Subscription::Unsubscribed => Subscription::Subscribed,
            },
        }),
    }
}

#[cfg(test)]
mod tests;
