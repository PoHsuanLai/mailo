//! What a caller changes: commands, applied in order on a thread of the client's own.
//!
//! A command is the same rule the command line applies ([`mail_core::act`]): an op on a
//! conversation, taking one back, deleting one for good. The caller is handed a [`Ticket`] at
//! once and never waits on the store's writer; the ticket says what was done, and the
//! [`Generation`] a value has to be read at to show it.
//!
//! What a caller shows meanwhile is its own: [`after`] says what an op will make of a
//! conversation, as near as its summary tells, so a caller can stop listing it at once.

use chrono::{DateTime, Utc};
use mail_core::undo::Undo;
use mail_domain::{
    Filter, LabelId, MailboxRole, MatchCtx, Membership, MessageId, Op, Target, ThreadId,
    ThreadSummary, View, ViewId,
};
use mail_store::{Generation, SqliteStore, Store};
use porter_core::AccountId;

/// What to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Apply each op to its conversation, oldest first. `clock` is how the words that say what
    /// was done write a time.
    Perform {
        ops: Vec<(ThreadId, Op)>,
        clock: mail_core::when::Clock,
        /// Conversations whose op reaches only the one message named, not the conversation whole:
        /// a message opened alone from a list that shows a message to a row.
        alone: Vec<(ThreadId, MessageId)>,
        /// Whether what an op moved to Trash is erased from there once the server holds it
        /// ([`mail_core::erase::mark_deleted`]), for a Delete that means "deletes now".
        then_erase: bool,
    },
    /// Take back each of these, newest first.
    TakeBack(Vec<Undo>),
    /// Delete for good what each conversation holds in Trash or Spam.
    Destroy(Vec<ThreadId>),
    /// Keep something the person made or forgot.
    Keep(Keep),
}

/// What a person makes or forgets, written like an op is: off the thread that draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keep {
    /// A saved view, new or changed.
    View(View),
    /// A saved view, forgotten.
    ForgetView(ViewId),
    /// A label named for an account.
    Label { account: AccountId, name: String },
}

/// What a [`Command`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// The ops that were applied, each with what takes it back. One that was refused is not here.
    Performed(Vec<(ThreadId, Op, Undo)>),
    /// What was taken back, and what was refused, in the order they were asked for.
    TookBack { back: Vec<Undo>, refused: Vec<Undo> },
    /// The messages deleted.
    Destroyed(Vec<MessageId>),
    /// What a [`Keep`] came to: the label it made, for one that makes a label, else `None`; or
    /// why it was not kept.
    Kept(Result<Option<LabelId>, String>),
}

/// A command done: what it did, and the generation a value read at or after has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acted {
    pub done: Done,
    pub generation: Generation,
}

/// What a command will have done, once it has. Awaited, or looked at without waiting.
#[derive(Debug)]
pub struct Ticket(tokio::sync::oneshot::Receiver<Acted>);

impl Ticket {
    pub(crate) fn new() -> (tokio::sync::oneshot::Sender<Acted>, Ticket) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (tx, Ticket(rx))
    }

    /// What was done, if it has been; `None` while it waits.
    pub fn now(&mut self) -> Option<Acted> {
        self.0.try_recv().ok()
    }
}

impl std::future::Future for Ticket {
    /// `None` when the client stopped before the command was done.
    type Output = Option<Acted>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx).map(Result::ok)
    }
}

/// Do `command` on `store`, here.
pub(crate) fn apply(store: &SqliteStore, command: Command) -> Acted {
    let done = match command {
        Command::Perform {
            ops,
            clock,
            alone,
            then_erase,
        } => {
            let done: Vec<(ThreadId, Op, Undo)> = ops
                .into_iter()
                .filter_map(|(thread, op)| {
                    let target = match alone.iter().find(|(of, _)| *of == thread) {
                        Some((_, message)) => Target::Messages(vec![*message]),
                        None => Target::Threads(vec![thread]),
                    };
                    mail_core::act::perform_on(store, thread, target, op.clone(), clock)
                        .map(|undo| (thread, op, undo))
                })
                .collect();
            if then_erase && done.iter().any(|(_, op, _)| *op == Op::Trash) {
                let undos: Vec<Undo> = done.iter().map(|(_, _, undo)| undo.clone()).collect();
                mail_core::erase::mark_deleted(store, &undos, Utc::now());
            }
            Done::Performed(done)
        }
        Command::TakeBack(undos) => {
            let (back, refused) = undos
                .into_iter()
                .partition(|undo| mail_core::act::take_back(store, undo));
            Done::TookBack { back, refused }
        }
        Command::Destroy(threads) => Done::Destroyed(
            threads
                .into_iter()
                .filter_map(|thread| mail_core::act::destroy(store, thread))
                .flatten()
                .collect(),
        ),
        Command::Keep(keep) => Done::Kept(
            match keep {
                Keep::View(view) => store.put_view(&view).map(|()| None),
                Keep::ForgetView(id) => store.delete_view(id).map(|()| None),
                Keep::Label { account, name } => store.create_label(account, &name).map(Some),
            }
            .map_err(|why| why.to_string()),
        ),
    };
    Acted {
        done,
        generation: store.generation(),
    }
}

/// What `op` will make of the conversation `summary` sums up, as near as the summary tells:
/// where it is filed, whether it is read, starred, labelled, snoozed, pinned. `None` when it
/// will be gone.
///
/// Near, not exact: an op acts on each message, and a summary is their union, so a message the
/// op leaves where it is (a copy in Sent, kept when the rest is archived) is not seen here.
pub fn after(summary: &ThreadSummary, op: &Op) -> Option<ThreadSummary> {
    let mut next = summary.clone();
    let filed = |keep: &[MailboxRole], into: MailboxRole| {
        summary
            .mailboxes
            .iter()
            .filter(|role| keep.contains(role))
            .fold(mail_domain::MailboxSet::only(into), |set, role| {
                set.with(role)
            })
    };
    match op {
        Op::Archive => next.mailboxes = filed(&[MailboxRole::Sent], MailboxRole::Archive),
        Op::Trash => next.mailboxes = filed(&[MailboxRole::Sent], MailboxRole::Trash),
        Op::Spam => next.mailboxes = filed(&[], MailboxRole::Spam),
        Op::Restore => next.mailboxes = filed(&[MailboxRole::Sent], MailboxRole::Inbox),
        Op::File(label) => {
            next.mailboxes = summary.mailboxes.without(MailboxRole::Inbox);
            if !next.labels.contains(label) {
                next.labels.push(*label);
            }
        }
        Op::SetRead(read) => next.read = *read,
        Op::SetStar(star) => next.star = *star,
        Op::Flag(hue) => {
            next.star = mail_domain::Star::Starred;
            next.flag = *hue;
        }
        Op::Copy(label) => {
            if !next.labels.contains(label) {
                next.labels.push(*label);
            }
        }
        Op::Label(label, Membership::In) => {
            if !next.labels.contains(label) {
                next.labels.push(*label);
            }
        }
        Op::Label(label, Membership::Out) => next.labels.retain(|it| it != label),
        Op::SetSnooze(snooze) => next.snooze = *snooze,
        Op::SetPin(pin) => next.pin = *pin,
        Op::SetMute(mute) => next.mute = *mute,
        Op::SetFollowUp(follow_up) => next.follow_up = *follow_up,
        Op::Destroy => return None,
    }
    Some(next)
}

/// Whether `op` takes the conversation `summary` sums up out of what `filter` lists at `now`:
/// it is listed now, and will not be. A filter the summary alone cannot answer (words in the
/// mail, a server folder, who the VIPs are) is never said to lose it, so a caller that hides on this hides
/// nothing it should have kept.
pub fn leaves(filter: &Filter, summary: &ThreadSummary, op: &Op, now: DateTime<Utc>) -> bool {
    let fits = |summary: &ThreadSummary| {
        filter.fit(&MatchCtx {
            summary,
            corpus: None,
            folders: &[],
            sizes: &[],
            cc: &[],
            vips: &[],
            now,
        })
    };
    fits(summary) && after(summary, op).is_none_or(|next| !fits(&next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::testing::summary;
    use mail_domain::{LabelId, ReadState, Snooze, Star};

    fn inbox() -> Filter {
        Filter::InMailbox(MailboxRole::Inbox)
    }

    #[test]
    fn what_an_op_takes_out_of_a_list() {
        let now = mail_domain::testing::now();
        let label = LabelId::generate();
        let unread = Filter::Read(ReadState::Unread);
        let starred = Filter::Starred(Star::Starred);
        let starred_row = ThreadSummary {
            star: Star::Starred,
            ..summary()
        };
        let snoozed_out = Filter::Not(Box::new(Filter::Snoozed));
        let words = Filter::Text(mail_domain::TextMatch::Contains("lunch".into()));
        let cases: Vec<(&str, Filter, ThreadSummary, Op, bool)> = vec![
            (
                "archived out of the inbox",
                inbox(),
                summary(),
                Op::Archive,
                true,
            ),
            (
                "trashed out of the inbox",
                inbox(),
                summary(),
                Op::Trash,
                true,
            ),
            (
                "filed out of the inbox",
                inbox(),
                summary(),
                Op::File(label),
                true,
            ),
            ("deleted for good", inbox(), summary(), Op::Destroy, true),
            (
                "starred, still in the inbox",
                inbox(),
                summary(),
                Op::SetStar(Star::Starred),
                false,
            ),
            (
                "read out of Unread",
                unread,
                summary(),
                Op::SetRead(ReadState::Read),
                true,
            ),
            (
                "unstarred out of Starred",
                starred.clone(),
                starred_row,
                Op::SetStar(Star::Unstarred),
                true,
            ),
            (
                "never listed, so not taken out",
                starred,
                summary(),
                Op::Archive,
                false,
            ),
            (
                "snoozed out of a list that leaves snoozed mail out",
                snoozed_out,
                summary(),
                Op::SetSnooze(Snooze::Until(now + chrono::TimeDelta::days(1))),
                true,
            ),
            (
                "words the summary cannot answer",
                words,
                summary(),
                Op::Archive,
                false,
            ),
            (
                "the VIP list, which the summary does not carry",
                Filter::Vip,
                summary(),
                Op::Archive,
                false,
            ),
        ];
        for (what, filter, row, op, leaving) in cases {
            assert_eq!(leaves(&filter, &row, &op, now), leaving, "{what}");
        }
    }

    #[test]
    fn a_copy_in_sent_stays_when_the_rest_is_archived() {
        let row = ThreadSummary {
            mailboxes: mail_domain::MailboxSet::only(MailboxRole::Inbox).with(MailboxRole::Sent),
            ..summary()
        };
        let archived = after(&row, &Op::Archive).expect("still there");
        assert!(archived.mailboxes.contains(MailboxRole::Sent));
        assert!(archived.mailboxes.contains(MailboxRole::Archive));
        assert!(!archived.mailboxes.contains(MailboxRole::Inbox));
    }
}
