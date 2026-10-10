//! What a conversation offers and what a gesture does to it: the decisions behind a row's menu,
//! the keyboard and Reply, with no window in them.
//!
//! The window asks these and draws the answer. Which actions a row offers, which operation a
//! button performs, which message a reply answers and how mute turns on a selection are about the
//! conversation and not about how it is shown, so they live here where a table can test them.

use mail_domain::{
    LabelId, MailboxRole, Membership, Message, Mute, Op, OpKind, ReadState, Star, ThreadId,
    ThreadSummary, View,
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
}
