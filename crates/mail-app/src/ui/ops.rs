use crate::ui::view::Shell;
use dioxus::prelude::{ReadableExt, Signal, WritableExt};
use mail_core::compose::{Answer, Quote};
use mail_core::conversation::{mute_op, op_for, reply_target};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_core::AccountId;

/// The composer pane.
///
/// Every field writes straight back into `Shell.composing`, and every button goes through
/// `mail_core::compose`, which is the same module the CLI calls. Two code paths for "send this
/// draft" is how a window and a command start disagreeing about what a draft is.
/// What opening a composer on this action means.
///
/// Three of the row's buttons open a composer rather than performing an operation. A forward is
/// not a reply with a different audience — it carries the message rather than answering it, and
/// it starts with no recipients — so it is a variant here rather than a third `ReplyScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Composes {
    Reply(ReplyScope),
    /// A reply to the list the message came through, at its `List-Post` address.
    ReplyList,
    Forward,
    /// A forward that carries the message itself as a `message/rfc822` attachment.
    ForwardAttached,
    /// Edit as New, and Send Again: a new message filled in from the conversation's newest.
    Again,
}

impl Composes {
    /// Whether this answers the message, so a selection in it is what it quotes.
    fn answers(self) -> bool {
        matches!(self, Composes::Reply(_) | Composes::ReplyList)
    }
}

pub(super) fn composes(kind: OpKind) -> Option<Composes> {
    match kind {
        OpKind::Reply => Some(Composes::Reply(ReplyScope::Sender)),
        OpKind::ReplyAll => Some(Composes::Reply(ReplyScope::All)),
        OpKind::Forward => Some(Composes::Forward),
        _ => None,
    }
}

/// Create the draft a reply button opens.
///
/// Which message that answers is [`mail_core::conversation::reply_target`]'s decision, not this function's.
/// Open a composer on the newest message of `thread`, replying or forwarding.
/// Begin a message that answers nothing.
///
/// `known` is the shell's own list, so the common case — one account — asks the store nothing
/// and the multi-account case picks a default the user can see and change in the From row.
/// Empty only before the first `use_effect` has run or before an account exists, and the store
/// is asked then, because a window that has just been opened should still be able to write.
pub(super) fn start_new(
    store: &SqliteStore,
    known: &[(String, AccountId)],
) -> Result<Draft, String> {
    let account = match known.first() {
        Some((_, id)) => id.clone(),
        None => mail_core::compose::account_for(store, None)?,
    };
    // No recipients and no subject: there is no original to take either from, and a guess is
    // something the sender has to notice and undo. Saved anyway, so closing the window keeps it.
    mail_core::compose::draft_new(store, account, &[], "", "", chrono::Utc::now())
        .map_err(String::from)
}

/// What to open the composer on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Writes {
    /// A message that answers nothing: the pencil, ⌘N and ⌘K's New Message.
    New,
    /// A reply or forward of a conversation: the row's buttons, `r`, `a`, `f` and ⌘K's forward.
    On(ThreadId, Composes),
    /// A reply or forward of the one message of a conversation the reader shows, opened from a
    /// list of single messages: the message answered is that one, not the newest.
    OnMessage(ThreadId, MessageId, Composes),
    /// A draft already stored: its row in Drafts, ⌘K's Recent, a `mailto:` link.
    Draft(DraftId),
}

impl Writes {
    /// A reply or forward of `thread`: of the message the reader shows alone when `thread` is the
    /// open conversation and was opened from a message's own row ([`Shell::focus`]), else of the
    /// conversation.
    pub(super) fn answering(shell: &Shell, thread: ThreadId, what: Composes) -> Self {
        match shell.focus {
            Some(message) if shell.open == Some(thread) => Self::OnMessage(thread, message, what),
            _ => Self::On(thread, what),
        }
    }
}

/// Open the composer on `what`. With no account to send from, a new message says so and offers
/// Add Account (`no_account`); anything else that stops it goes to the log. The refusal comes
/// back for a caller that says it somewhere of its own; `Ok` when the composer opened.
pub(super) fn open_composer(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    what: Writes,
) -> Result<(), String> {
    let opened = match what {
        Writes::New => {
            let known = shell.peek().accounts.clone();
            let started = start_new(store, &known);
            if let Err(why) = &started
                && known.is_empty()
                && mail_core::compose::sending_accounts(store).is_empty()
            {
                shell.write().no_account = true;
                return Err(why.clone());
            }
            started
                .map(|draft| copied(store, draft))
                .map(|draft| shell.write().compose(&draft))
        }
        Writes::On(thread, composes) | Writes::OnMessage(thread, _, composes) => {
            let only = match what {
                Writes::OnMessage(_, message, _) => Some(message),
                _ => None,
            };
            // Read here, as the press is heard: the selection is the window's, in its frames.
            let selected = composes
                .answers()
                .then(|| store.thread(thread).ok())
                .flatten()
                .and_then(|loaded| crate::ui::reading::selection::selected(&loaded.messages));
            start_composing_on(store, thread, only, composes, selected)
                .map(|draft| copied(store, draft))
                .map(|draft| shell.write().compose(&draft))
        }
        Writes::Draft(id) => shell.write().compose_id(store, id),
    };
    if let Err(why) = &opened {
        log::warn!("compose: {why}");
    }
    opened
}

/// `draft` with the person's own address on its Cc or Bcc, as `compose.copy_myself` asks; as it
/// was when the setting is off, or when the copy could not be written (the message is still
/// worth writing without it, and the composer shows its Cc and Bcc).
fn copied(store: &SqliteStore, draft: Draft) -> Draft {
    use crate::settings::CopyMyself;
    let to = match crate::ui::prefs::current().compose.copy_myself {
        CopyMyself::Off => return draft,
        CopyMyself::Cc => mail_core::compose::Myself::Cc,
        CopyMyself::Bcc => mail_core::compose::Myself::Bcc,
    };
    match mail_core::compose::copy_myself(store, draft.id, to) {
        Ok(copied) => copied,
        Err(why) => {
            log::warn!("compose: the copy to myself was not added: {why}");
            draft
        }
    }
}

/// Create the draft `what` opens on `thread`. `selected` is the text selected in one of its
/// messages, and that message: an answer answers it and quotes only that. `only` is the one
/// message to answer, forward or write again, when the reader shows one alone; the
/// conversation's newest otherwise.
pub(super) fn start_composing_on(
    store: &SqliteStore,
    thread: ThreadId,
    only: Option<MessageId>,
    what: Composes,
    selected: Option<(MessageId, String)>,
) -> Result<Draft, String> {
    match what {
        Composes::Reply(scope) => start_reply(store, thread, only, Answer::Scope(scope), selected),
        Composes::ReplyList => start_reply(store, thread, only, Answer::List, selected),
        Composes::Again => {
            let target = match only {
                Some(message) => message,
                None => newest(store, thread)?,
            };
            mail_core::compose::draft_again(store, target, chrono::Utc::now()).map_err(String::from)
        }
        Composes::Forward | Composes::ForwardAttached => {
            let target = match only {
                Some(message) => message,
                None => {
                    let loaded = store.thread(thread).map_err(|e| e.to_string())?;
                    let messages: Vec<Message> = loaded
                        .messages
                        .iter()
                        .filter_map(|id| store.message(*id).ok())
                        .collect();
                    reply_target(&messages)
                        .ok_or_else(|| "that conversation has no message to forward".to_owned())?
                        .id
                }
            };
            // No recipients: a forward has none of its own and the composer is where the user
            // names them. The draft is saved regardless, so closing the window does not lose it.
            let forwarded = match what {
                Composes::ForwardAttached => mail_core::compose::draft_forward_attached(
                    store,
                    target,
                    &[],
                    "",
                    chrono::Utc::now(),
                ),
                _ => mail_core::compose::draft_forward(store, target, &[], "", chrono::Utc::now()),
            };
            forwarded.map_err(String::from)
        }
    }
}

/// The message of `thread` Reply answers ([`reply_target`]).
pub(super) fn newest(store: &SqliteStore, thread: ThreadId) -> Result<MessageId, String> {
    let loaded = store.thread(thread).map_err(|e| e.to_string())?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    reply_target(&messages)
        .map(|target| target.id)
        .ok_or_else(|| "that conversation has no messages".to_owned())
}

fn start_reply(
    store: &SqliteStore,
    thread: ThreadId,
    only: Option<MessageId>,
    to: Answer,
    selected: Option<(MessageId, String)>,
) -> Result<Draft, String> {
    let (target, quote) = match (&selected, only) {
        (Some((message, text)), _) => (*message, Quote::Selection(text)),
        (None, Some(message)) => (message, Quote::Whole),
        (None, None) => (newest(store, thread)?, Quote::Whole),
    };
    // Where the signature goes is the person's setting: under the quoted original unless asked.
    let placement = super::prefs::current()
        .compose
        .signature_above_quote
        .placement();
    mail_core::compose::draft_answer(store, target, to, quote, chrono::Utc::now(), placement)
        .map_err(String::from)
}

/// Put a label on a conversation, or take it off, as a test does to set one up.
///
/// Separate from `apply_op` because `Op::Label` carries a payload no `OpKind` can supply — which
/// is exactly why the row's "Label" button opened nothing for as long as it existed.
#[cfg(test)]
pub(super) fn apply_label(
    store: &SqliteStore,
    thread: ThreadId,
    label: LabelId,
    membership: Membership,
) -> bool {
    apply(store, thread, Op::Label(label, membership))
}

/// Apply the op a button means for `thread`, as a test does to set one up: whether anything
/// changed.
#[cfg(test)]
pub(super) fn apply_op(store: &SqliteStore, thread: ThreadId, kind: OpKind) -> bool {
    resolve(store, thread, kind).is_some_and(|op| apply(store, thread, op))
}

/// The operation a button means for this conversation now.
///
/// `Pin` needs a payload `op_for` cannot supply — the direction comes from the conversation's
/// current state and the rank from the clock — so it is resolved here, where both are in
/// reach, and `Mute` likewise takes its direction from the conversation. Label and snooze open
/// a menu rather than acting, and resolve to nothing.
pub(super) fn resolve(store: &SqliteStore, thread: ThreadId, kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Pin => {
            let loaded = store.thread(thread).ok()?;
            Some(mail_core::mailbox_filter::pin_op(
                &loaded.summary,
                chrono::Utc::now(),
            ))
        }
        OpKind::Mute => {
            let loaded = store.thread(thread).ok()?;
            Some(mute_op(&loaded.summary))
        }
        other => op_for(other),
    }
}

#[cfg(test)]
fn apply(store: &SqliteStore, thread: ThreadId, op: Op) -> bool {
    perform(store, thread, op, mail_core::when::Clock::H24).is_some()
}

// What the window and `mailo intents` both do to conversations lives in the core
// (`mail_core::act`): two code paths for "archive this" is how two front-ends start disagreeing.
pub(super) use mail_core::act::caps_here;
#[cfg(test)]
pub(super) use mail_core::act::{perform, take_back};

#[cfg(test)]
mod tests {
    use super::apply_op;
    use crate::ui::fixtures::{acct_account, gmail_caps, inbox_query, realistic};
    use mail_core::{SqliteStore, Store};
    use mail_domain::*;

    /// What the server is told when the window acts.
    ///
    /// `Op::apply` returns the local patch, its undo, *and* the remote work. `apply_op` used
    /// the first and dropped the third, and built the `AccountCaps` it passed out of safe
    /// defaults rather than reading the account's own — and `remote_intent` is the only
    /// consumer of `caps`, returning `None` for Archive and Trash under `LocalOnly`. So the
    /// operation could not have produced remote work, and would have been discarded if it had.
    mod what_the_server_is_told {
        use super::*;

        fn first_thread(store: &SqliteStore) -> ThreadId {
            store
                .threads(&inbox_query(), chrono::Utc::now())
                .unwrap()
                .items
                .first()
                .expect("the fixture has mail")
                .id
        }

        #[test]
        fn archiving_in_the_window_reaches_the_server() {
            // The conversation used to be archived here and nowhere else: still in the inbox on
            // the phone, and back in this one on the next full sync.
            let (store, _dir) = realistic();
            let thread = first_thread(&store);

            assert!(apply_op(&store, thread, OpKind::Archive));

            let queued = store
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            let intents: Vec<&ProtoOp> = queued.iter().map(|entry| &entry.op).collect();
            assert!(
                intents.iter().any(|op| matches!(
                    op,
                    ProtoOp::SetMailbox {
                        role: MailboxRole::Archive,
                        ..
                    }
                )),
                "nothing was queued for the server: {intents:?}"
            );
        }

        #[test]
        fn marking_read_reaches_the_server_too() {
            // This one produces a remote intent whatever the capabilities say, so it was lost
            // purely to the dropped field — mail read here stayed bold everywhere else.
            let (store, _dir) = realistic();
            let thread = first_thread(&store);

            assert!(apply_op(&store, thread, OpKind::MarkRead));

            let queued = store
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                queued
                    .iter()
                    .any(|entry| matches!(&entry.op, ProtoOp::SetFlags { read: Some(_), .. })),
                "the flag never left this machine: {:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }

        #[test]
        fn an_account_that_files_nothing_queues_nothing() {
            // The control, and the reason this was invisible: on a POP3 account
            // `ArchiveMeans::LocalOnly` is the truth and archiving really is local. The account
            // tested against most was the one where the hardcoded value happened to be right.
            let (store, _dir) = realistic();
            let local_only = AccountCaps {
                labels: ServerLabels::LocalOnly,
                archive: ArchiveMeans::LocalOnly,
                ..gmail_caps()
            };
            mail_store::testing::seed_caps(&store, acct_account(), &local_only, chrono::Utc::now())
                .unwrap();
            let thread = first_thread(&store);

            assert!(apply_op(&store, thread, OpKind::Archive));
            let queued = store
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                !queued
                    .iter()
                    .any(|entry| matches!(&entry.op, ProtoOp::SetMailbox { .. })),
                "a POP3 account has nowhere to file anything"
            );
        }

        #[test]
        fn a_pin_has_no_server_half_and_queues_nothing() {
            // Pin and snooze are this application's own state. Queueing them would ask the
            // server to do something it has no word for.
            let (store, _dir) = realistic();
            let thread = first_thread(&store);

            assert!(apply_op(&store, thread, OpKind::Pin));
            let queued = store
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                queued.is_empty(),
                "{:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }
    }
}
