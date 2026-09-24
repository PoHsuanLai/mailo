//! What the window can take back, and how to say what it did.
//!
//! An entry is an [`Applied`]'s undo kept beside the patch it undoes. Undoing writes the
//! inverse through the store like any other patch, and — when the forward op told the server —
//! queues the reverse for the server too, so an archive undone here is not still archived on
//! the phone.
//!
//! [`Applied`]: mail_domain::Applied

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{
    AccountId, Change, FolderWork, LabelId, MailboxRole, Membership, MessageId, NonEmpty, Op,
    Patch, Pin, ReadState, RemoteIntent, Snooze, Star, Subscription, ThreadId,
};
use std::collections::BTreeMap;

/// How many operations the window remembers.
pub const DEPTH: usize = 20;

/// One operation that can be taken back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undo {
    /// What the toast said: "Archived".
    pub said: String,
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
}

/// The most recent operations, newest last.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndoStack {
    entries: Vec<Undo>,
}

impl UndoStack {
    /// Remember `entry`, forgetting the oldest past [`DEPTH`].
    pub fn push(&mut self, entry: Undo) {
        self.entries.push(entry);
        if self.entries.len() > DEPTH {
            self.entries.remove(0);
        }
    }

    /// Take the newest entry.
    pub fn pop(&mut self) -> Option<Undo> {
        self.entries.pop()
    }

    /// The newest entry, left in place.
    pub fn last(&self) -> Option<&Undo> {
        self.entries.last()
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
pub fn reverse_intent(remote: &RemoteIntent, inverse: &Patch) -> Option<RemoteIntent> {
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
        RemoteIntent::SetFlags { .. } => {
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
        RemoteIntent::Send { .. }
        | RemoteIntent::AddKeyword { .. }
        | RemoteIntent::Append { .. }
        | RemoteIntent::File { .. } => None,
    }
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

/// What the toast says an operation did.
pub fn said<Tz: TimeZone>(op: &Op, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match op {
        Op::Archive => "Archived".to_owned(),
        Op::Trash => "Moved to Trash".to_owned(),
        Op::Restore => "Moved to Inbox".to_owned(),
        Op::Spam => "Marked as spam".to_owned(),
        Op::SetRead(ReadState::Read) => "Marked read".to_owned(),
        Op::SetRead(ReadState::Unread) => "Marked unread".to_owned(),
        Op::SetStar(Star::Starred) => "Starred".to_owned(),
        Op::SetStar(Star::Unstarred) => "Unstarred".to_owned(),
        Op::Label(_, Membership::In) => "Labelled".to_owned(),
        Op::Label(_, Membership::Out) => "Label removed".to_owned(),
        Op::SetSnooze(Snooze::Until(at)) => format!("Snoozed until {}", when(*at, zone)),
        Op::SetSnooze(Snooze::Inactive) => "Back in the inbox".to_owned(),
        Op::SetPin(Pin::Rank(_)) => "Pinned".to_owned(),
        Op::SetPin(Pin::Unpinned) => "Unpinned".to_owned(),
        Op::File(_) => "Moved to folder".to_owned(),
    }
}

fn when<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.with_timezone(zone).format("%a %H:%M").to_string()
}

#[cfg(test)]
mod tests;
