//! What a mailbox place lists, shared by the window and the command line's `list`.
//!
//! Listing them, with their unread counts and the selected row, is the window's
//! (`mail_ui::ui::view`); the filters here are the part both front-ends must agree on.

use chrono::{DateTime, Utc};
use mail_domain::{Filter, MailboxRole, Op, Pin, ThreadSummary};

/// What an unread count counts in `place`: its conversations that are unread.
///
/// One definition for the places' unread counts and the launcher's count, so they cannot disagree about
/// what "unread in the inbox" means.
pub fn unread_in(place: Filter) -> Filter {
    Filter::And(vec![place, Filter::Read(mail_domain::ReadState::Unread)])
}

/// The filter a mailbox place lists.
///
/// Public and shared, because there were two of these and they disagreed. The shell asked
/// `source_for`, the CLI's `list` built `Filter::InMailbox(role)` itself, and the day the inbox
/// learned to hide a snoozed conversation only one of them learned it — so `mailo list` went on
/// showing what the window had put away. One definition, both callers.
///
/// `Drafts` is not here: drafts are not threads and no `Filter` selects one. See [`Source`].
pub fn for_role(role: MailboxRole) -> Filter {
    match role {
        // A snoozed conversation is *away* until its instant passes; that is the whole of what
        // snoozing means, and a client that leaves it in the list has a snooze that does
        // nothing. Only the inbox hides them — Archive and Search still show everything,
        // because a thread you snoozed is not a thread you lost.
        MailboxRole::Inbox => Filter::And(vec![
            Filter::InMailbox(MailboxRole::Inbox),
            Filter::Not(Box::new(pending_snooze())),
        ]),
        // Archived is filed in Archive with nothing in the inbox (`mail_core::conversation::Whereabouts`).
        // A thread's mailboxes are the union over its messages, and on Gmail All Mail is filed
        // as Archive and holds the inbox's mail too, so "has a message in Archive" listed every
        // inbox conversation here as well, and its menu offered Archive to what looked archived.
        MailboxRole::Archive => Filter::And(vec![
            Filter::InMailbox(MailboxRole::Archive),
            Filter::Not(Box::new(Filter::InMailbox(MailboxRole::Inbox))),
        ]),
        other => Filter::InMailbox(other),
    }
}

/// The mailbox a place's filter lists, when it is one of [`for_role`]'s: its inverse.
pub fn role_of(filter: &Filter) -> Option<MailboxRole> {
    MailboxRole::ALL
        .into_iter()
        .find(|role| for_role(*role) == *filter)
}

/// What a click on the pin does to a conversation, given what it is now.
///
/// A toggle, and the rank is the moment it was pinned, so the most recently pinned sits first
/// when they are ordered by it. `Op::SetPin` needs a payload, which is why `op_for` cannot
/// produce it and this can: the current state decides the direction and the clock decides the
/// rank.
pub fn pin_op(summary: &ThreadSummary, now: DateTime<Utc>) -> Op {
    match summary.pin {
        Pin::Rank(_) => Op::SetPin(Pin::Unpinned),
        Pin::Unpinned => Op::SetPin(Pin::Rank(now.timestamp())),
    }
}

/// Snoozed, and not yet due.
///
/// `SnoozeDue` implies `Snoozed`, so "away" is the difference between them rather than a state
/// of its own — which is what `filter.rs` means by predicates rather than state mirrors. Due is
/// resolved against `now` at query time, so a thread returns to the inbox on the stroke without
/// anything having to run.
pub fn pending_snooze() -> Filter {
    Filter::And(vec![
        Filter::Snoozed,
        Filter::Not(Box::new(Filter::SnoozeDue)),
    ])
}
