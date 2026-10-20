//! Delete Forever, and Empty Trash: what the window offers, and what its alert says.
//!
//! The user's rule: mail is deleted for good only from Trash or Spam, and only after an alert
//! that names how much and says it cannot be undone. Empty Trash waits for that place to be what
//! the window shows; Delete Forever is a conversation's own, once it is in one of them. Everywhere else, deleting is a move to Trash. [`mail_domain::Op::Destroy`] refuses any
//! message outside Trash and Spam whatever asks it; this module is the other half, which decides
//! when the window asks at all. Pure: the alert and its buttons are `ui::destroy`.

use super::view::{Shell, Source};
use mail_core::place::place_filter;
use mail_domain::{MailboxRole, Message, ThreadId, ThreadSummary};

/// A place mail is deleted forever from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bin {
    Trash,
    Spam,
}

impl Bin {
    /// The mailbox it is.
    pub fn role(self) -> MailboxRole {
        match self {
            Bin::Trash => MailboxRole::Trash,
            Bin::Spam => MailboxRole::Spam,
        }
    }

    /// Its name in the sidebar, which the alert and the buttons use.
    pub fn name(self) -> &'static str {
        match self {
            Bin::Trash => "Trash",
            Bin::Spam => "Spam",
        }
    }
}

/// The bin the window is showing: the Trash or Spam place, with no search over it. `None`
/// anywhere else, including a search run from Trash, whose results are not Trash's.
pub fn bin_shown(shell: &Shell) -> Option<Bin> {
    if !shell.search.trim().is_empty() {
        return None;
    }
    let Source::Mail(filter) = &shell.places.get(shell.selected)?.source else {
        return None;
    };
    [Bin::Trash, Bin::Spam]
        .into_iter()
        .find(|bin| *filter == place_filter(bin.role()))
}

/// Whether `summary` has mail in `bin`, the bin shown, so Delete Forever reaches it from there.
pub fn offered(bin: Option<Bin>, summary: &ThreadSummary) -> bool {
    bin.is_some_and(|bin| summary.mailboxes.contains(bin.role()))
}

/// How many of `messages` a Delete Forever from `bin` removes. The rest of a conversation — a
/// reply in Sent, a copy archived — stays.
pub fn doomed(messages: &[Message], bin: Bin) -> usize {
    messages.iter().filter(|m| m.mailbox == bin.role()).count()
}

/// What the confirmation is asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The conversations picked, or the one row pressed.
    Chosen,
    /// Everything the bin holds in the accounts shown: Empty Trash.
    Everything,
}

/// The confirmation alert while it is open: what it deletes, counted when it was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destroying {
    pub bin: Bin,
    pub reach: Reach,
    pub threads: Vec<ThreadId>,
    /// Messages in the bin among `threads`.
    pub messages: usize,
}

/// What the alert says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    pub body: String,
    /// The button that deletes.
    pub confirm: String,
}

/// The alert's words: how much, from where, and that it cannot be undone.
pub fn words(destroying: &Destroying) -> Words {
    let bin = destroying.bin.name();
    let conversations = destroying.threads.len();
    let title = match (destroying.reach, conversations) {
        (Reach::Everything, _) => format!("Empty {bin}?"),
        (Reach::Chosen, 1) => "Delete this conversation forever?".to_owned(),
        (Reach::Chosen, n) => format!("Delete {n} conversations forever?"),
    };
    let messages = match destroying.messages {
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    };
    let body = format!(
        "{messages} in {bin} will be deleted from this computer and the server. {}",
        crate::ui::copy::confirm::CANT_BE_UNDONE
    );
    let confirm = match destroying.reach {
        Reach::Everything => format!("Empty {bin}"),
        Reach::Chosen => "Delete Forever".to_owned(),
    };
    Words {
        title,
        body,
        confirm,
    }
}

/// What the toast says once it is done. No Undo goes with it.
pub fn said(messages: usize) -> String {
    match messages {
        1 => "Deleted forever · 1 message".to_owned(),
        n => format!("Deleted forever · {n} messages"),
    }
}

#[cfg(test)]
mod tests;
