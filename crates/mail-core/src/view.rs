//! What a conversation offers and what a gesture does to it: the decisions behind a row's menu,
//! the keyboard and Reply, with no window in them.
//!
//! The window asks these and draws the answer. Which actions a row offers, which operation a
//! button performs, which message a reply answers and how mute turns on a selection are about the
//! conversation and not about how it is shown, so they live here where a table can test them.

use crate::place::{pending_snooze, place_filter};
use chrono::{DateTime, Utc};
use mail_domain::{
    Filter, FollowUp, LabelId, MailboxRef, MailboxRole, MailboxSet, Membership, Message, Mute, Op,
    OpKind, Pin, ReadState, Snooze, Star, ThreadId, ThreadSummary, View,
};

/// What Mute does to a conversation, given what it is now: mute it, or unmute it if it is muted.
///
/// `Op::SetMute` carries the state it sets, so `op_for` cannot produce it; the conversation's own
/// state decides the direction.
pub fn mute_op(summary: &ThreadSummary) -> Op {
    Op::SetMute(match summary.mute {
        Mute::Muted => Mute::Unmuted,
        Mute::Unmuted => Mute::Muted,
    })
}

/// The one mute a gesture applies to several conversations: unmute when every one of them is
/// muted, mute otherwise. One operation for all, never a toggle each, as star and read on a
/// selection are.
pub fn mute_for_all(summaries: &[ThreadSummary]) -> Mute {
    if !summaries.is_empty() && summaries.iter().all(|s| s.mute == Mute::Muted) {
        Mute::Unmuted
    } else {
        Mute::Muted
    }
}

/// Where a conversation is, as the person sees it: one answer for a thread whose messages are
/// in several mailboxes.
///
/// A thread's `mailboxes` is the union over its messages, so it can say Inbox and Archive at
/// once, and that is common: on Gmail `[Gmail]/All Mail` is filed as Archive and holds every
/// message, the inbox's too, and on any server a reply into an archived conversation arrives in
/// the inbox beside the archived ones. The rule, in this order:
///
/// 1. **Inbox** when any message is in the inbox. That is what the inbox lists and what a new
///    reply means everywhere: the conversation is back. Being in All Mail (or an Archive folder)
///    as well is not being archived.
/// 2. **Trash**, then **Spam**, when any message is in one: what is in a bin is on its way out,
///    and the way back or Delete Forever is what it needs, not another throw.
/// 3. **Archive** when any message is filed there and none is in the inbox or a bin.
/// 4. **Elsewhere**: only Sent or Drafts, the person's own mail that was never filed.
///
/// The Archive place lists by the same rule ([`crate::place::place_filter`]), so what it lists is
/// always offered Move to Inbox and never Archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whereabouts {
    Inbox,
    Trash,
    Spam,
    Archive,
    Elsewhere,
}

impl Whereabouts {
    pub fn of(mailboxes: MailboxSet) -> Self {
        if mailboxes.contains(MailboxRole::Inbox) {
            Whereabouts::Inbox
        } else if mailboxes.contains(MailboxRole::Trash) {
            Whereabouts::Trash
        } else if mailboxes.contains(MailboxRole::Spam) {
            Whereabouts::Spam
        } else if mailboxes.contains(MailboxRole::Archive) {
            Whereabouts::Archive
        } else {
            Whereabouts::Elsewhere
        }
    }
}

/// The word an offered action is shown with: which side of a two-sided action it is.
///
/// The window names each by its kind (`ui::copy::menus`); this is the key. A conversation is
/// never offered the side it is already on: an unread one is offered Mark as Read, a pinned one
/// Unpin, an archived one Move to Inbox, one in Spam Not Spam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Say {
    Archive,
    /// Restore, out of Archive or Trash.
    MoveToInbox,
    /// Restore, out of Spam.
    NotSpam,
    Spam,
    Trash,
    DeleteForever,
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Pin,
    Unpin,
    Mute,
    Unmute,
    /// Opens the times to snooze until.
    Snooze,
    /// Snoozed and not yet due: bring it back now.
    Unsnooze,
    /// Opens the times to be reminded at.
    Remind,
    /// A reminder set or come back: let it go.
    StopReminding,
    Label,
    Reply,
    ReplyAll,
    Forward,
}

/// The side `kind` is offered as for this conversation at `now`.
///
/// Read and star are two kinds, and [`hover_actions`] picks the one the conversation needs.
/// Restore, pin, mute, snooze and the follow-up are one kind each whose direction the
/// conversation's state decides, and this names that direction. `now` is for the snooze alone:
/// one whose time has passed is back, and is offered Snooze again.
pub fn says(kind: OpKind, summary: &ThreadSummary, now: DateTime<Utc>) -> Say {
    match kind {
        OpKind::Archive => Say::Archive,
        OpKind::Restore => match Whereabouts::of(summary.mailboxes) {
            Whereabouts::Spam => Say::NotSpam,
            _ => Say::MoveToInbox,
        },
        OpKind::Spam => Say::Spam,
        OpKind::Trash => Say::Trash,
        OpKind::Destroy => Say::DeleteForever,
        OpKind::MarkRead => Say::MarkRead,
        OpKind::MarkUnread => Say::MarkUnread,
        OpKind::Star => Say::Star,
        OpKind::Unstar => Say::Unstar,
        OpKind::Pin => match summary.pin {
            Pin::Rank(_) => Say::Unpin,
            Pin::Unpinned => Say::Pin,
        },
        OpKind::Mute => match summary.mute {
            Mute::Muted => Say::Unmute,
            Mute::Unmuted => Say::Mute,
        },
        OpKind::Snooze if away(summary, now) => Say::Unsnooze,
        OpKind::Snooze => Say::Snooze,
        OpKind::FollowUp => match summary.follow_up {
            FollowUp::Inactive => Say::Remind,
            FollowUp::Until { .. } | FollowUp::Returned { .. } => Say::StopReminding,
        },
        OpKind::AddLabel | OpKind::RemoveLabel => Say::Label,
        OpKind::Reply => Say::Reply,
        OpKind::ReplyAll => Say::ReplyAll,
        OpKind::Forward => Say::Forward,
    }
}

/// The side a selection is offered `kind` as: the side every conversation that is offered it
/// shares, else the first one's. `None` when none of them is offered it.
///
/// For a bar over several conversations: Not Spam when everything it reaches is in Spam, Move to
/// Inbox when some of it is archived.
pub fn says_for_all(kind: OpKind, summaries: &[ThreadSummary], now: DateTime<Utc>) -> Option<Say> {
    let mut sides = summaries
        .iter()
        .filter(|summary| offers(summary, kind))
        .map(|summary| says(kind, summary, now));
    let first = sides.next()?;
    let generic = match kind {
        OpKind::Restore => Say::MoveToInbox,
        _ => first,
    };
    Some(if sides.all(|side| side == first) {
        first
    } else {
        generic
    })
}

/// Snoozed, and its time not yet come.
fn away(summary: &ThreadSummary, now: DateTime<Utc>) -> bool {
    matches!(summary.snooze, Snooze::Until(at) if at > now)
}

/// What picking `kind` does at once when it is offered as the way out of a state the
/// conversation is in: Unsnooze and Stop Reminding. `None` when it opens its menu of times
/// instead, and for every other kind.
pub fn turns_off(kind: OpKind, summary: &ThreadSummary, now: DateTime<Utc>) -> Option<Op> {
    match says(kind, summary, now) {
        Say::Unsnooze => Some(Op::SetSnooze(Snooze::Inactive)),
        Say::StopReminding => Some(Op::SetFollowUp(FollowUp::Inactive)),
        _ => None,
    }
}

/// What a row offers for a thread, in its menu, the reader's ⋯ and the keyboard.
///
/// Derived from where the thread is ([`Whereabouts`]) and its state, one side of each pair:
/// Archive in the inbox, Move to Inbox out of Archive or Trash, Not Spam out of Spam (both
/// [`OpKind::Restore`]), Trash anywhere but Trash, Spam anywhere but Spam, and read and star as
/// the conversation needs them. Pin, mute, snooze, the follow-up, a label and a forward are
/// always offered; [`says`] names which way the first four go. Returns [`OpKind`] rather than
/// [`Op`] because a menu item cannot carry a payload that does not exist yet.
pub fn hover_actions(summary: &ThreadSummary) -> Vec<OpKind> {
    let here = Whereabouts::of(summary.mailboxes);
    let mut out = Vec::new();
    match here {
        Whereabouts::Inbox => out.push(OpKind::Archive),
        Whereabouts::Archive | Whereabouts::Trash | Whereabouts::Spam => out.push(OpKind::Restore),
        Whereabouts::Elsewhere => {}
    }
    if here != Whereabouts::Spam {
        out.push(OpKind::Spam);
    }
    if here != Whereabouts::Trash {
        out.push(OpKind::Trash);
    }
    out.push(match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    });
    out.push(match summary.star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    });
    // Always offered, each by the side the conversation needs (`says`). None depends on where
    // the conversation is: a forward is the message being passed on, a pin is a note to
    // yourself about it, a mute is about the replies still to come, a snooze and a reminder are
    // about when you see it next, and a label is a name you are giving it.
    out.push(OpKind::Pin);
    out.push(OpKind::Mute);
    out.push(OpKind::Snooze);
    out.push(OpKind::FollowUp);
    out.push(OpKind::AddLabel);
    out.push(OpKind::Forward);
    out
}

/// What a row's menu offers for a thread in `view`: the view's own actions where it names any,
/// else [`hover_actions`].
///
/// A view names kinds, not directions, so its Star or Mark read is drawn as whichever of the
/// pair the conversation needs, and a button the conversation cannot take (Archive on one not in
/// the inbox) is left out, as [`hover_actions`] leaves it out. The keyboard is not narrowed by a
/// view: [`offers`] still asks [`hover_actions`].
pub fn hover_in(view: Option<&View>, summary: &ThreadSummary) -> Vec<OpKind> {
    let offered = hover_actions(summary);
    let Some(view) = view.filter(|view| !view.hover.is_empty()) else {
        return offered;
    };
    let mut out: Vec<OpKind> = Vec::new();
    for kind in &view.hover {
        let kind = match kind {
            OpKind::Star | OpKind::Unstar => match summary.star {
                Star::Unstarred => OpKind::Star,
                Star::Starred => OpKind::Unstar,
            },
            OpKind::MarkRead | OpKind::MarkUnread => match summary.read {
                ReadState::Unread => OpKind::MarkRead,
                ReadState::Read => OpKind::MarkUnread,
            },
            other => *other,
        };
        let allowed = match kind {
            // Neither needs a place to be allowed from; a reply is a draft.
            OpKind::Reply | OpKind::ReplyAll => true,
            other => offers(summary, other),
        };
        if allowed && !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// One row of the label menu: the name, which label it is, and whether this conversation has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChoice {
    pub name: String,
    pub id: LabelId,
    pub membership: Membership,
}

impl LabelChoice {
    /// What clicking it should do — the opposite of where the conversation is now.
    pub fn toggled(&self) -> Membership {
        self.membership.flip()
    }
}

/// The label menu for one conversation.
///
/// `known` is the shell's own index, so this stays a pure function of what is on screen. Names
/// are not unique — `UNIQUE (account, name)` is per account, so "travel" on two accounts is two
/// labels — and both are listed rather than merged, because giving a conversation on one account
/// the other account's "travel" is not something this menu can do and not something it should imply.
///
/// Sorted by name, with the ones already on the conversation first: the common act is taking a
/// label off the thing you are looking at, and it should not be a search.
pub fn label_menu(known: &[(String, LabelId)], summary: &ThreadSummary) -> Vec<LabelChoice> {
    let mut out: Vec<LabelChoice> = known
        .iter()
        .map(|(name, id)| LabelChoice {
            name: name.clone(),
            id: *id,
            membership: if summary.labels.contains(id) {
                Membership::In
            } else {
                Membership::Out
            },
        })
        .collect();
    out.sort_by(|a, b| {
        let on = |c: &LabelChoice| c.membership == Membership::In;
        on(b)
            .cmp(&on(a))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Whether `kind` is something this conversation can have done to it now: what its row offers
/// ([`hover_actions`]).
pub fn offers(summary: &ThreadSummary, kind: OpKind) -> bool {
    hover_actions(summary).contains(&kind)
}

/// The conversation `Next` or `Previous` moves to.
///
/// Does not wrap. A list that jumps from the bottom back to the top loses the user's place in a
/// way that is hard to notice and easy to act on — the next keystroke archives the wrong thing.
/// Nothing open means the end you are coming from: the first row going down, the last going up.
pub fn step(current: Option<ThreadId>, ids: &[ThreadId], forward: bool) -> Option<ThreadId> {
    if ids.is_empty() {
        return None;
    }
    let Some(here) = current.and_then(|id| ids.iter().position(|it| *it == id)) else {
        // Nothing open, or something that is no longer in the list — archived out from under
        // the selection, most often. Start from the end the movement comes from.
        return Some(if forward { ids[0] } else { ids[ids.len() - 1] });
    };
    let next = if forward {
        here.checked_add(1).filter(|i| *i < ids.len())
    } else {
        here.checked_sub(1)
    };
    Some(ids[next.unwrap_or(here)])
}

/// The `Op` a hover button performs, where it needs no payload.
///
/// `None` for the ones that open a composer instead — a reply is a draft, not an operation.
pub fn op_for(kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Archive => Some(Op::Archive),
        OpKind::Trash => Some(Op::Trash),
        OpKind::Restore => Some(Op::Restore),
        OpKind::Spam => Some(Op::Spam),
        OpKind::MarkRead => Some(Op::SetRead(ReadState::Read)),
        OpKind::MarkUnread => Some(Op::SetRead(ReadState::Unread)),
        OpKind::Star => Some(Op::SetStar(Star::Starred)),
        OpKind::Unstar => Some(Op::SetStar(Star::Unstarred)),
        // These need a label picked, a draft created, a date chosen, or the conversation's own
        // state to say which way they go. Delete forever needs the confirmation sheet, which
        // applies it itself: no row, key or bar performs it on a press (the window's bin).
        OpKind::Destroy
        | OpKind::AddLabel
        | OpKind::RemoveLabel
        | OpKind::Snooze
        | OpKind::Pin
        | OpKind::Mute
        | OpKind::FollowUp
        | OpKind::Reply
        | OpKind::ReplyAll
        | OpKind::Forward => None,
    }
}

/// The message a reply to this thread should answer.
///
/// The newest, which is what "reply" means to everyone except the person who wrote the
/// threading code. Replying to the thread's *root* would quote a conversation's opening line
/// back at someone who has since sent four more, and would set `In-Reply-To` to a message the
/// recipient's client threads above everything they last read.
///
/// Ties break on id so a thread with two messages at the same timestamp — which a bulk import
/// produces easily — picks the same one every time rather than alternating between renders.
pub fn reply_target(messages: &[Message]) -> Option<&Message> {
    messages.iter().max_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    })
}

/// An entry in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    /// What this place lists. Not a `Filter`: see [`Source`].
    pub source: Source,
    /// Shown as a badge. `None` until counted, which is not the same as zero.
    pub unread: Option<u64>,
}

/// The default sidebar.
///
/// Inbox is `Filter::InMailbox(Inbox)` rather than anything special, which is the plan's claim
/// that a place is just a saved filter — made true here rather than asserted.
/// Where a sidebar place gets its rows.
///
/// Two variants because drafts are genuinely not mail yet. A draft has no thread, no server
/// address and no mailbox — it lives in its own table — so there is no `Filter` that selects
/// one, and a `Drafts` place built from `Filter::InMailbox(MailboxRole::Drafts)` lists nothing,
/// for ever, with no error. That was the state of this sidebar until the composer gave it
/// something to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Threads matching a filter.
    Mail(Filter),
    /// The drafts table.
    Drafts,
    /// A view the user saved: its filter, and how its list is grouped and what its rows offer.
    ///
    /// The whole [`View`] rather than its filter, because the rest of it is what makes it a
    /// view: `Shell::grouping` and [`hover_in`] read it while it is the place shown.
    Saved(Box<View>),
    /// The conversations with a follow-up reminder, soonest due first ([`mail_core::follow_up`]).
    ///
    /// Not a `Filter`, for the reason drafts are not: whether a reminder still stands is decided
    /// on the conversation's messages and the user's own addresses, and the domain's filters see
    /// only the summary. The store lists them ([`mail_store::Store::follow_ups`]).
    Waiting,
    /// Every conversation the person opened, newest open first ([`crate::history`]).
    ///
    /// Not a `Filter` either: what was opened and when is the store's own record
    /// ([`mail_store::Store::opened`]), which no summary carries.
    History,
}

fn source_for(role: MailboxRole) -> Source {
    match role {
        MailboxRole::Drafts => Source::Drafts,
        other => Source::Mail(place_filter(other)),
    }
}

pub fn default_places() -> Vec<Place> {
    // Starred sits with the places that ask "what is this mail", ahead of the folders.
    // Snoozed stays with them: a conversation that was put off is looked for there, not beside
    // Trash. Pinned threads stay a place of their own, distinct from a Space's pinned people.
    [
        ("Inbox", source_for(MailboxRole::Inbox)),
        ("Starred", Source::Mail(Filter::Starred(Star::Starred))),
        (
            "Snoozed",
            // Only the ones still away. A due thread is back in the inbox, and showing it here
            // as well would make "snoozed" mean two different things in two places.
            Source::Mail(pending_snooze()),
        ),
        ("Archive", source_for(MailboxRole::Archive)),
        ("Sent", source_for(MailboxRole::Sent)),
        ("Drafts", source_for(MailboxRole::Drafts)),
        ("Spam", source_for(MailboxRole::Spam)),
        ("Trash", source_for(MailboxRole::Trash)),
        ("Pinned", Source::Mail(Filter::Pinned)),
        // Last, so every place before it keeps the index it had: conversations the user is
        // waiting on an answer to, which come back to the inbox if none arrives.
        ("Waiting", Source::Waiting),
        // After Waiting, for the same reason: the places before it keep their index.
        ("History", Source::History),
    ]
    .into_iter()
    .map(|(name, source)| Place {
        name: name.to_owned(),
        source,
        unread: None,
    })
    .collect()
}

/// A place that lists one label, as opposed to a mailbox.
pub fn is_label_place(place: &Place) -> bool {
    matches!(place.source, Source::Mail(Filter::HasLabel(_)))
}

/// The filter a server folder's place lists: the account's threads with a message the server
/// holds at exactly `path`.
///
/// For a folder with no role and no label behind it. The inbox and the role places list by
/// role, and where folders are labels (Gmail) a folder is listed through its label.
pub fn folder_filter(mailbox: &MailboxRef) -> Filter {
    Filter::And(vec![
        Filter::Account(mailbox.account.clone()),
        Filter::InFolder(mailbox.clone()),
    ])
}

/// The folder a place lists, when it is a server folder's place.
pub fn folder_of(place: &Place) -> Option<&MailboxRef> {
    match &place.source {
        Source::Mail(Filter::And(parts)) => match parts.as_slice() {
            [Filter::Account(account), Filter::InFolder(mailbox)]
                if *account == mailbox.account =>
            {
                Some(mailbox)
            }
            _ => None,
        },
        _ => None,
    }
}

/// The sidebar: the default places, then one per label, then one per server folder, then one
/// per saved view, in that order — which is also the order the badges are counted in, index for
/// index.
///
/// A folder's place is named by its last level, which is what its row and the list's title say.
/// Saved views come last so that keeping or forgetting one moves no other place's index.
pub fn places_with(
    labels: &[(String, LabelId)],
    folders: &[(String, MailboxRef)],
    views: &[View],
) -> Vec<Place> {
    let labelled = labels.iter().map(|(name, id)| Place {
        name: name.clone(),
        source: Source::Mail(Filter::HasLabel(*id)),
        unread: None,
    });
    let foldered = folders.iter().map(|(name, mailbox)| Place {
        name: name.clone(),
        source: Source::Mail(folder_filter(mailbox)),
        unread: None,
    });
    default_places()
        .into_iter()
        .chain(labelled)
        .chain(foldered)
        .chain(views.iter().map(saved_place))
        .collect()
}

/// The sidebar place a saved view is.
pub fn saved_place(view: &View) -> Place {
    Place {
        name: view.name.clone(),
        source: Source::Saved(Box::new(view.clone())),
        unread: None,
    }
}

/// The saved view a place is, when it is one.
pub fn saved_of(place: &Place) -> Option<&View> {
    match &place.source {
        Source::Saved(view) => Some(view),
        Source::Mail(_) | Source::Drafts | Source::Waiting | Source::History => None,
    }
}

/// What a place's badge counts, or `None` when it has no badge.
///
/// Unread threads, and only for mail: "3 unread drafts" is not a thing, because a draft is not
/// something that arrives and is not something anyone has failed to read yet.
///
/// Sent and Archive get one too. That looks odd until a filter rule files an unread message
/// straight into Archive, at which point a badgeless Archive is a message the user never learns
/// about.
/// `filter` narrowed to `scope`, the accounts the window is showing (`None` is every account).
///
/// The one narrowing the list and the sidebar's counts both go through, so a badge counts what
/// opening its place lists: a count over every account beside a list of one said Trash held a
/// message that was in another account's Trash.
pub fn scoped(scope: Option<Filter>, filter: Filter) -> Filter {
    match scope {
        Some(scope) => Filter::And(vec![scope, filter]),
        None => filter,
    }
}

/// What a place's badge counts in `scope`: its unread, among the accounts its list shows.
pub fn scoped_badge(source: &Source, scope: Option<Filter>) -> Option<Filter> {
    badge_filter(source).map(|filter| scoped(scope, filter))
}

pub fn badge_filter(source: &Source) -> Option<Filter> {
    match source {
        Source::Mail(filter) => Some(crate::place::unread_in(filter.clone())),
        Source::Saved(view) => Some(crate::place::unread_in(view.filter.clone())),
        // Nothing in it is news: the user wrote the last word and is waiting for someone else's.
        Source::Drafts | Source::Waiting | Source::History => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_domain::id::new_account_id;
    use mail_domain::*;

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: mail_domain::FollowUp::Inactive,
        };
        tweak(&mut s);
        s
    }

    #[test]
    fn hover_actions_fit_where_the_thread_is() {
        let inbox = hover_actions(&summary(|_| {}));
        assert!(inbox.contains(&OpKind::Archive), "{inbox:?}");
        assert!(!inbox.contains(&OpKind::Restore), "nothing to restore from");

        let archived = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Archive)
        }));
        assert!(archived.contains(&OpKind::Restore), "{archived:?}");
        assert!(
            !archived.contains(&OpKind::Archive),
            "offering to archive what is archived is noise"
        );

        let trashed = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Trash)
        }));
        assert!(!trashed.contains(&OpKind::Trash), "{trashed:?}");
    }

    /// The owner's case: a conversation the Archive lists, right-clicked, offered Archive.
    ///
    /// A thread's mailboxes are the union over its messages. On Gmail `[Gmail]/All Mail` is
    /// filed as Archive and holds the inbox's mail too, and on any server a reply into an
    /// archived conversation lands in the inbox beside the archived messages; either way the
    /// thread says Inbox and Archive at once. Each case is listed by the Archive place or not, and
    /// a conversation it lists must be offered the way back and not Archive.
    #[test]
    fn what_the_archive_lists_is_offered_move_to_inbox_and_not_archive() {
        let set = |roles: &[MailboxRole]| {
            roles
                .iter()
                .fold(MailboxSet::empty(), |set, role| set.with(*role))
        };
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let cases: &[(&str, &[MailboxRole])] = &[
            (
                "Gmail: in the inbox, its All Mail copy filed as Archive",
                &[MailboxRole::Inbox, MailboxRole::Archive],
            ),
            (
                "Gmail: archived, in All Mail alone",
                &[MailboxRole::Archive],
            ),
            ("IMAP: moved to the Archive folder", &[MailboxRole::Archive]),
            (
                "IMAP: a reply came into an archived conversation",
                &[MailboxRole::Archive, MailboxRole::Inbox],
            ),
            (
                "IMAP: archived, with my reply in Sent",
                &[MailboxRole::Archive, MailboxRole::Sent],
            ),
        ];
        let archive = place_filter(MailboxRole::Archive);
        for (case, roles) in cases {
            let thread = summary(|s| s.mailboxes = set(roles));
            let listed = archive.fit(&mail_domain::filter::MatchCtx {
                summary: &thread,
                corpus: None,
                folders: &[],
                now,
            });
            if !listed {
                continue;
            }
            let offered = hover_actions(&thread);
            assert!(!offered.contains(&OpKind::Archive), "{case}: {offered:?}");
            assert!(offered.contains(&OpKind::Restore), "{case}: {offered:?}");
        }
    }

    /// Every two-sided action against both of its states, in every place: exactly one side is
    /// offered, and it is named by the conversation's state.
    #[test]
    fn every_pair_is_offered_by_the_side_the_conversation_is_not_on() {
        use MailboxRole::*;
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        // Where the conversation's messages are, and the sides of Archive / Move to Inbox,
        // Spam / Not Spam and Trash it is offered there.
        type Place<'a> = (&'a str, &'a [MailboxRole], &'a [Say]);
        let places: &[Place] = &[
            (
                "in the inbox",
                &[Inbox],
                &[Say::Archive, Say::Spam, Say::Trash],
            ),
            (
                "Gmail: in the inbox, its All Mail copy filed as Archive",
                &[Inbox, Archive],
                &[Say::Archive, Say::Spam, Say::Trash],
            ),
            (
                "Gmail: archived, in All Mail alone",
                &[Archive],
                &[Say::MoveToInbox, Say::Spam, Say::Trash],
            ),
            (
                "IMAP: in the Archive folder, with my reply in Sent",
                &[Archive, Sent],
                &[Say::MoveToInbox, Say::Spam, Say::Trash],
            ),
            (
                "IMAP: a reply came into an archived conversation",
                &[Archive, Inbox],
                &[Say::Archive, Say::Spam, Say::Trash],
            ),
            ("in Spam", &[Spam], &[Say::NotSpam, Say::Trash]),
            ("in Trash", &[Trash], &[Say::MoveToInbox, Say::Spam]),
            (
                "one message trashed out of an archived conversation",
                &[Archive, Trash],
                &[Say::MoveToInbox, Say::Spam],
            ),
            (
                "only my own mail, in Sent",
                &[Sent],
                &[Say::Spam, Say::Trash],
            ),
        ];
        let removal = [
            Say::Archive,
            Say::MoveToInbox,
            Say::NotSpam,
            Say::Spam,
            Say::Trash,
        ];
        // The conversation's own state, and the side of read, star, pin, mute, snooze and the
        // follow-up it is offered.
        type State<'a> = (&'a str, fn(&mut ThreadSummary), [Say; 6]);
        let states: &[State] = &[
            (
                "unread, nothing set",
                |_| {},
                [
                    Say::MarkRead,
                    Say::Star,
                    Say::Pin,
                    Say::Mute,
                    Say::Snooze,
                    Say::Remind,
                ],
            ),
            (
                "read, starred, pinned, muted, snoozed and waiting on a reply",
                |s| {
                    s.read = ReadState::Read;
                    s.star = Star::Starred;
                    s.pin = Pin::Rank(1);
                    s.mute = Mute::Muted;
                    s.snooze = Snooze::Until(Utc.timestamp_opt(1_700_100_000, 0).unwrap());
                    s.follow_up = FollowUp::Until {
                        at: Utc.timestamp_opt(1_700_100_000, 0).unwrap(),
                        set: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
                    };
                },
                [
                    Say::MarkUnread,
                    Say::Unstar,
                    Say::Unpin,
                    Say::Unmute,
                    Say::Unsnooze,
                    Say::StopReminding,
                ],
            ),
            (
                "its snooze come due, its reminder come back",
                |s| {
                    s.snooze = Snooze::Until(Utc.timestamp_opt(1_600_000_000, 0).unwrap());
                    s.follow_up = FollowUp::Returned {
                        at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
                        set: Utc.timestamp_opt(1_500_000_000, 0).unwrap(),
                    };
                },
                [
                    Say::MarkRead,
                    Say::Star,
                    Say::Pin,
                    Say::Mute,
                    Say::Snooze,
                    Say::StopReminding,
                ],
            ),
        ];
        for (place, roles, sides) in places {
            for (state, tweak, wanted) in states {
                let thread = summary(|s| {
                    s.mailboxes = roles
                        .iter()
                        .fold(MailboxSet::empty(), |set, role| set.with(*role));
                    tweak(s);
                });
                let said: Vec<Say> = hover_actions(&thread)
                    .into_iter()
                    .map(|kind| says(kind, &thread, now))
                    .collect();
                let removed: Vec<Say> = said
                    .iter()
                    .copied()
                    .filter(|say| removal.contains(say))
                    .collect();
                assert_eq!(removed, *sides, "{place}, {state}");
                let kept: Vec<Say> = said
                    .iter()
                    .copied()
                    .filter(|say| !removal.contains(say))
                    .collect();
                let mut want = wanted.to_vec();
                want.extend([Say::Label, Say::Forward]);
                assert_eq!(kept, want, "{place}, {state}");
            }
        }
    }

    #[test]
    fn a_way_out_of_a_state_is_one_pick_and_the_way_in_opens_its_times() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let soon = Utc.timestamp_opt(1_700_100_000, 0).unwrap();
        let fresh = summary(|_| {});
        let snoozed = summary(|s| s.snooze = Snooze::Until(soon));
        let reminding = summary(|s| s.follow_up = FollowUp::Until { at: soon, set: now });
        assert_eq!(turns_off(OpKind::Snooze, &fresh, now), None);
        assert_eq!(turns_off(OpKind::FollowUp, &fresh, now), None);
        assert_eq!(
            turns_off(OpKind::Snooze, &snoozed, now),
            Some(Op::SetSnooze(Snooze::Inactive))
        );
        assert_eq!(
            turns_off(OpKind::FollowUp, &reminding, now),
            Some(Op::SetFollowUp(FollowUp::Inactive))
        );
        // Due, it is back: offered Snooze again, which asks when.
        assert_eq!(turns_off(OpKind::Snooze, &snoozed, soon), None);
    }

    #[test]
    fn a_selection_is_offered_not_spam_only_when_all_of_it_is_spam() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let spam = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Spam));
        let archived = summary(|s| s.mailboxes = MailboxSet::only(MailboxRole::Archive));
        let inbox = summary(|_| {});
        let restore = |all: &[ThreadSummary]| says_for_all(OpKind::Restore, all, now);
        assert_eq!(restore(&[spam.clone(), inbox.clone()]), Some(Say::NotSpam));
        assert_eq!(restore(&[spam, archived.clone()]), Some(Say::MoveToInbox));
        assert_eq!(restore(&[archived]), Some(Say::MoveToInbox));
        assert_eq!(restore(&[inbox]), None);
    }

    #[test]
    fn actions_needing_a_payload_have_no_bare_op() {
        // A reply is a draft, not an operation, so a hover button cannot perform one.
        for kind in [
            OpKind::Reply,
            OpKind::Forward,
            OpKind::AddLabel,
            OpKind::Snooze,
        ] {
            assert!(op_for(kind).is_none(), "{kind:?} should open something");
        }
        assert_eq!(op_for(OpKind::Archive), Some(Op::Archive));
    }

    fn message(n: i64) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: new_account_id(),
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap(),
            from: Address {
                name: None,
                email: format!("s{n}@example.test"),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: format!("subject {n}"),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn reply_target_is_the_newest_or_none() {
        // Replying to the root quotes a conversation's opening line back at someone who has
        // since sent four more, and threads the reply above everything they last read. An empty
        // thread is reachable: the messages are loaded one by one and any of them can fail to
        // read.
        let cases: [(&str, &[i64], Option<&str>); 2] = [
            (
                "the newest, out of order",
                &[0, 300, 100],
                Some("subject 300"),
            ),
            ("an empty thread", &[], None),
        ];
        for (name, dates, want) in cases {
            let messages: Vec<Message> = dates.iter().map(|n| message(*n)).collect();
            let got = reply_target(&messages).map(|target| target.subject.as_str());
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn the_choice_is_stable_when_two_messages_share_a_timestamp() {
        // A bulk import produces these easily. An unstable pick would reply to a different
        // message on each render, which the user would see as the quote changing under them.
        let messages = vec![message(5), message(5), message(5)];
        let first = reply_target(&messages).unwrap().id;
        for _ in 0..8 {
            assert_eq!(reply_target(&messages).unwrap().id, first);
        }
    }

    #[test]
    fn mute_turns_by_what_the_conversations_are() {
        let unmuted = summary(|_| {});
        let muted = summary(|s| s.mute = Mute::Muted);
        assert_eq!(mute_op(&unmuted), Op::SetMute(Mute::Muted));
        assert_eq!(mute_op(&muted), Op::SetMute(Mute::Unmuted));
        // One mute for a selection: unmute only when every one is muted, and never for none.
        assert_eq!(mute_for_all(&[]), Mute::Muted);
        assert_eq!(mute_for_all(&[muted.clone(), muted.clone()]), Mute::Unmuted);
        assert_eq!(mute_for_all(&[muted, unmuted]), Mute::Muted);
    }

    #[test]
    fn labels_on_the_conversation_come_first_then_by_name() {
        let (travel, bills, alpha) = (
            LabelId::generate(),
            LabelId::generate(),
            LabelId::generate(),
        );
        let known = vec![
            ("travel".to_owned(), travel),
            ("Bills".to_owned(), bills),
            ("alpha".to_owned(), alpha),
        ];
        let thread = summary(|s| s.labels = vec![travel]);
        let menu = label_menu(&known, &thread);
        let order: Vec<&str> = menu.iter().map(|choice| choice.name.as_str()).collect();
        assert_eq!(order, ["travel", "alpha", "Bills"]);
        assert_eq!(menu[0].toggled(), Membership::Out);
    }

    #[test]
    fn the_sidebar_offers_somewhere_to_find_a_snoozed_conversation() {
        // Otherwise snoozing is a way to lose mail.
        let places = default_places();
        let snoozed = places
            .iter()
            .find(|p| p.name == "Snoozed")
            .expect("no Snoozed place");
        assert_eq!(snoozed.source, Source::Mail(pending_snooze()));
    }
}

#[cfg(test)]
mod badge_tests {
    use super::*;

    #[test]
    fn a_mail_place_counts_its_unread_threads() {
        let inbox = Source::Mail(Filter::InMailbox(MailboxRole::Inbox));
        match badge_filter(&inbox) {
            Some(Filter::And(clauses)) => {
                assert!(clauses.contains(&Filter::InMailbox(MailboxRole::Inbox)));
                assert!(
                    clauses.contains(&Filter::Read(ReadState::Unread)),
                    "the badge counted every thread, not the unread ones: {clauses:?}"
                );
            }
            other => panic!("expected a conjunction, got {other:?}"),
        }
    }

    #[test]
    fn every_mail_place_gets_one_including_archive() {
        // A filter rule can file an unread message straight into Archive. A badgeless Archive is
        // then a message the user never finds out about.
        for place in default_places() {
            match &place.source {
                Source::Mail(_) | Source::Saved(_) => assert!(
                    badge_filter(&place.source).is_some(),
                    "{} has no badge",
                    place.name
                ),
                // The user wrote the last word in each of them: nothing there is unread news.
                // "3 unread drafts" is not a thing: a draft did not arrive and nobody failed to
                // read it.
                Source::Drafts | Source::Waiting | Source::History => assert_eq!(
                    badge_filter(&place.source),
                    None,
                    "{} has a badge",
                    place.name
                ),
            }
        }
    }
}
