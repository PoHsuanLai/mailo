//! What a conversation offers and what a gesture does to it: the decisions behind a row's menu,
//! the keyboard and Reply, with no window in them.
//!
//! The window asks these and draws the answer. Which actions a row offers, which operation a
//! button performs, which message a reply answers and how mute turns on a selection are about the
//! conversation and not about how it is shown, so they live here where a table can test them.

use crate::place::{pending_snooze, place_filter};
use mail_domain::{
    Filter, LabelId, MailboxRef, MailboxRole, Membership, Message, Mute, Op, OpKind, ReadState,
    Star, ThreadId, ThreadSummary, View,
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

/// What a row offers for a thread, in its menu (once the hover strip's buttons; the name stays).
///
/// Derived from where the thread is, so Archive does not offer "archive" and Trash offers
/// "restore". Returns [`OpKind`] rather than [`Op`] because a menu item cannot carry a payload
/// that does not exist yet.
pub fn hover_actions(summary: &ThreadSummary) -> Vec<OpKind> {
    let mut out = Vec::new();
    if summary.mailboxes.contains(MailboxRole::Inbox) {
        out.push(OpKind::Archive);
    }
    if summary.mailboxes.contains(MailboxRole::Trash)
        || summary.mailboxes.contains(MailboxRole::Archive)
        || summary.mailboxes.contains(MailboxRole::Spam)
    {
        out.push(OpKind::Restore);
    }
    if !summary.mailboxes.contains(MailboxRole::Trash) {
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
    // Always offered. None depends on where the conversation is or what state it is in: a
    // forward is the message being passed on, a pin is a note to yourself about it, a mute is
    // about the replies still to come, and a label is a name you are giving it.
    out.push(OpKind::Pin);
    out.push(OpKind::Mute);
    out.push(OpKind::Snooze);
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
/// ([`hover_actions`]), and Spam, which the row has no room for, for anything not already there.
pub fn offers(summary: &ThreadSummary, kind: OpKind) -> bool {
    match kind {
        OpKind::Spam => !summary.mailboxes.contains(MailboxRole::Spam),
        other => hover_actions(summary).contains(&other),
    }
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
        Source::Mail(_) | Source::Drafts | Source::Waiting => None,
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
pub fn badge_filter(source: &Source) -> Option<Filter> {
    match source {
        Source::Mail(filter) => Some(Filter::And(vec![
            filter.clone(),
            Filter::Read(ReadState::Unread),
        ])),
        Source::Saved(view) => Some(Filter::And(vec![
            view.filter.clone(),
            Filter::Read(ReadState::Unread),
        ])),
        // Nothing in it is news: the user wrote the last word and is waiting for someone else's.
        Source::Drafts | Source::Waiting => None,
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
                Source::Drafts | Source::Waiting => assert_eq!(
                    badge_filter(&place.source),
                    None,
                    "{} has a badge",
                    place.name
                ),
            }
        }
    }
}
