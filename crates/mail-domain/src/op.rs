//! What the user does, and the local mutations it produces.
//!
//! # Why `Applied::remote` is always `None` today
//!
//! [`Op::apply`] is handed a thread and its messages. Every addressable field of a
//! [`ProtoOp`] that a user op could produce -- `SetFlags`, `SetMailbox`, `SetLabels`,
//! `Expunge` -- is addressed by [`crate::RemoteRef`], and a `RemoteRef` is not reachable from
//! a [`Message`]: the `(account, mailbox, uidvalidity, uid)` mapping is the `remote_map`
//! table in `mail-store`, and it is many-to-one, so it cannot be recomputed here even in
//! principle. `ProtoOp::SetLabels` needs the same thing twice over: its `add`/`remove` are
//! server-side label *names*, while [`Op::Label`] carries a [`LabelId`] and this crate has no
//! label table to resolve it against.
//!
//! So `apply` cannot build a complete `ProtoOp`, and it does not build an incomplete one.
//! `Some(ProtoOp::SetFlags { remotes: vec![], .. })` would enqueue an operation that touches
//! zero messages -- precisely the "queued no-op" the capability branches exist to avoid --
//! and it would be indistinguishable in the outbox from a genuinely local-only account.
//! Until the interface grows a way to express "this op, against these `MessageId`s, for the
//! store to resolve", the honest value is `None` and the remote work is the caller's to
//! build. `caps` is unused for exactly the same reason: no capability changes the *local*
//! mutation, only the remote one. This is reported as a frozen-interface finding rather than
//! worked around here.

use crate::account::{AccountCaps, ArchiveMeans, ServerLabels};
use crate::content::Label;
use crate::draft::Draft;
use crate::folder::{Folder, FolderWork};
use crate::id::{BlobId, ChangeId, DraftId, LabelId, MessageId, ThreadId};
use crate::message::{Message, Thread};
use crate::receipt::Keyword;
use crate::remote::MailboxRef;
use crate::state::{MailboxRole, Membership, Pin, ReadState, Snooze, Star};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What an operation acts on. Plural: "mark 40 as read" is one action with one undo entry
/// and one queued [`ProtoOp`], not forty of each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Target {
    Threads(Vec<ThreadId>),
    Messages(Vec<MessageId>),
}

/// An operation the user can perform.
///
/// Reply, forward and send are deliberately absent. They create a [`Draft`], which has its own
/// lifecycle ([`crate::SendState`]) — folding them in here forced `Op::Reply(Compose)` into
/// saved views, where no draft exists yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Op {
    Archive,
    Trash,
    /// Out of Trash or Archive, back to Inbox.
    Restore,
    Spam,
    SetRead(ReadState),
    SetStar(Star),
    Label(LabelId, Membership),
    SetSnooze(Snooze),
    SetPin(Pin),
    /// Out of the inbox into the folder this label names: a label and an archive in one, which
    /// is what moving to a folder is where mailboxes are labels, and what a `MOVE` into that
    /// folder leaves behind locally everywhere else.
    ///
    /// One op rather than the two, because on a server with folders they are one command: a
    /// label and an archive queued apart would move the message into Archive and then find
    /// nothing left in the inbox to file.
    File(LabelId),
}

/// An operation with its payload stripped: "which action", without "that action's arguments".
///
/// What a hover strip, a keybinding or an undo label needs. Includes the draft-creating
/// actions, which are affordances rather than [`Op`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Archive,
    Trash,
    Restore,
    Spam,
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    AddLabel,
    RemoveLabel,
    Snooze,
    Pin,
    Reply,
    ReplyAll,
    Forward,
}

/// An operation bound to its targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub target: Target,
    pub op: Op,
}

/// One local mutation. Domain-level: a `Change` never mentions a table or a column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Change {
    MessageRead(MessageId, ReadState),
    MessageStar(MessageId, Star),
    MessageMailbox(MessageId, MailboxRole),
    MessageLabel(MessageId, LabelId, Membership),
    ThreadSnooze(ThreadId, Snooze),
    ThreadPin(ThreadId, Pin),
    MessageUpsert(Box<Message>),
    MessageDelete(MessageId),
    LabelUpsert(Label),
    DraftUpsert(Box<Draft>),
    DraftDelete(DraftId),
    /// A label and every message's membership of it. Membership is removed first by the
    /// caller, with [`Change::MessageLabel`], so that thread summaries are rebuilt from it.
    LabelRemove(LabelId),
    /// A mailbox as listed, added or replaced.
    FolderUpsert(Folder),
    /// A mailbox gone. Only the folder: its messages' server addresses are dropped when the
    /// server confirms the delete, because an undo cannot put them back.
    FolderRemove(MailboxRef),
    /// A mailbox and everything beneath it renamed, with every record keyed by those names:
    /// server addresses, sync cursors and, where mailboxes are labels, the labels.
    ///
    /// `delimiter` is the renamed folder's, which is what "beneath" is measured with.
    FolderRename {
        from: MailboxRef,
        to: String,
        delimiter: Option<char>,
    },
}

/// A set of mutations applied as one unit.
///
/// Distinct from [`crate::Ingest`] on purpose. A patch is small, invertible and optimistic; an
/// ingest is bulk, not invertible, and *is* the truth. One type for both produces a mega-enum
/// whose `apply` is a 300-line match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    pub id: ChangeId,
    pub changes: Vec<Change>,
}

/// Remote work implied by an [`Op`], addressed in **local** ids.
///
/// Not a [`ProtoOp`]. `Op::apply` cannot build one: every `ProtoOp` a user op could produce is
/// addressed by [`crate::RemoteRef`], and the local-id-to-`RemoteRef` mapping is `remote_map`
/// in `mail-store` — which `plan.md` is explicit is **many-to-one**, so it cannot be
/// recomputed here even in principle. `SetLabels` has a second, independent blocker: it needs
/// server-side label *names*, and this crate has no label table to resolve a [`LabelId`]
/// against.
///
/// So the domain states the *intent* and `Store::enqueue` resolves it into a `ProtoOp`, being
/// the only place where `remote_map` and `labels` both exist. `ProtoOp` itself is unchanged,
/// which matters: it is a persisted outbox schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum RemoteIntent {
    SetFlags {
        messages: Vec<MessageId>,
        read: Option<ReadState>,
        star: Option<Star>,
    },
    SetMailbox {
        messages: Vec<MessageId>,
        role: MailboxRole,
    },
    SetLabels {
        messages: Vec<MessageId>,
        add: Vec<LabelId>,
        remove: Vec<LabelId>,
    },
    /// Put a keyword on messages, for other clients to read.
    ///
    /// Add only. The one keyword this client sets, [`Keyword::MdnSent`], records that
    /// something happened, and nothing un-happens it; there is no local state to re-layer
    /// either, since the answer itself is kept by the store beside the message.
    AddKeyword {
        messages: Vec<MessageId>,
        keyword: Keyword,
    },
    /// Submit a composed message.
    ///
    /// The odd one out, and deliberately in the same queue as the rest. A send needs exactly
    /// what a flag change needs — ordering against the user's other actions, backoff on a
    /// transient refusal, and a record that survives a restart — and a second queue beside the
    /// outbox would be a second implementation of all three.
    ///
    /// It carries no `messages`: a submission does not address an existing message, it *is*
    /// the new one. Everything it needs was resolved when the user pressed send.
    Send {
        draft: DraftId,
        raw: BlobId,
        mail_from: String,
        rcpt_to: Vec<String>,
    },
    /// Create, rename, delete or follow a mailbox.
    ///
    /// Addresses no message, like [`RemoteIntent::Send`], so it resolves to the identical
    /// [`crate::ProtoOp::Folder`] without consulting `remote_map`.
    Folder(FolderWork),
    /// File messages into the folder a label names: out of the inbox, into that folder.
    ///
    /// Resolved by the store to [`crate::ProtoOp::File`] with the label's name, which is the
    /// folder's path.
    File {
        messages: Vec<MessageId>,
        label: LabelId,
    },
    /// Upload a message into a mailbox, e.g. imported mail.
    ///
    /// Addresses no existing message either: the bytes are the new one. Resolves to the
    /// identical [`crate::ProtoOp::Append`].
    Append {
        mailbox: MailboxRef,
        flags: Vec<crate::remote::SystemFlag>,
        date: Option<DateTime<Utc>>,
        raw: BlobId,
    },
}

/// The result of applying an [`Op`], or of planning a folder change with [`crate::folder::plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// What to write locally, right now.
    pub forward: Patch,
    /// What to write to undo it.
    ///
    /// Computed here rather than by a standalone `Op::invert()`, because an inverse needs the
    /// *prior* state: undoing `Label(x, In)` on a thread that already had `x` is a no-op, and
    /// undoing `Archive` depends on which mailboxes the thread was in.
    pub inverse: Patch,
    /// Remote work to queue, or `None` when this op is purely local — archiving under
    /// [`crate::ArchiveMeans::LocalOnly`], labelling under
    /// [`crate::ServerLabels::LocalOnly`], and snooze and pin always, which have no server
    /// representation at all.
    pub remote: Option<RemoteIntent>,
}

impl Op {
    /// Apply this operation, producing the local mutation, its undo, and any remote work.
    ///
    /// Pure. `messages` must be every message of `thread`, because the derived fields of
    /// [`crate::ThreadSummary`] have to be rebuilt and a thread-level op fans out across them.
    ///
    /// `forward` and `inverse` each carry a **fresh** [`ChangeId`]. A `ChangeId` names one
    /// application event, not a reversible pair: the store records it in `pending_changes` to
    /// re-layer the change over incoming server truth, and applying the undo is a second,
    /// separate event that must be recordable, re-layerable and itself undoable. Sharing one
    /// id would make "which patch is this row" ambiguous the moment an undo is applied, and
    /// would collide with the already-recorded forward patch.
    ///
    /// `now` is unused. No op currently needs a timestamp: [`Snooze::Until`] carries the
    /// instant the caller chose, and "has this snooze expired" is resolved against `now` at
    /// query time by [`crate::Filter::SnoozeDue`] rather than frozen in at apply time. The
    /// parameter stays because it is the crate-wide convention (`CONVENTIONS.md` section 6)
    /// and because an op that does need one -- a send-later, a reminder -- is a plausible
    /// addition.
    pub fn apply(
        &self,
        target: &Target,
        thread: &Thread,
        messages: &[Message],
        caps: &AccountCaps,
        _now: DateTime<Utc>,
    ) -> Applied {
        let selected = select(target, messages);
        let id = thread.summary.id;

        // Snooze and pin are thread-level, so they need "is this thread in scope" rather than
        // a message list. Naming the thread targets it; naming any of its messages does too,
        // because there is no such thing as snoozing half a conversation.
        let thread_targeted = match target {
            Target::Threads(ids) => ids.contains(&id),
            Target::Messages(_) => !selected.is_empty(),
        };

        // Each `Change` is independent of the others -- one message's read flag, one message's
        // mailbox -- so `inverse` is built in the same order as `forward` rather than reversed.
        let (forward, inverse) = match self {
            Op::Archive => move_into(&selected, MailboxRole::Archive),
            Op::Trash => move_into(&selected, MailboxRole::Trash),
            Op::Spam => move_into(&selected, MailboxRole::Spam),
            Op::Restore => restore(&selected),
            Op::SetRead(state) => set_read(&selected, *state),
            Op::SetStar(star) => set_star(&selected, *star),
            Op::Label(label, membership) => set_label(&selected, *label, *membership),
            Op::File(label) => file_into(&selected, *label),
            Op::SetSnooze(snooze) => {
                let prior = thread.summary.snooze;
                if thread_targeted && prior != *snooze {
                    (
                        vec![Change::ThreadSnooze(id, *snooze)],
                        vec![Change::ThreadSnooze(id, prior)],
                    )
                } else {
                    (Vec::new(), Vec::new())
                }
            }
            Op::SetPin(pin) => {
                let prior = thread.summary.pin;
                if thread_targeted && prior != *pin {
                    (
                        vec![Change::ThreadPin(id, *pin)],
                        vec![Change::ThreadPin(id, prior)],
                    )
                } else {
                    (Vec::new(), Vec::new())
                }
            }
        };

        // Only changed messages need syncing: a no-op locally is a no-op remotely, and
        // queueing it would cost a round trip to tell the server what it already believes.
        //
        // Each message once, in the order first touched: `File` changes a message twice (its
        // mailbox and its label), and naming it twice would address it twice on the wire.
        let mut touched: Vec<MessageId> = Vec::new();
        for change in &forward {
            if let Change::MessageRead(m, _)
            | Change::MessageStar(m, _)
            | Change::MessageMailbox(m, _)
            | Change::MessageLabel(m, _, _) = change
                && !touched.contains(m)
            {
                touched.push(*m);
            }
        }

        let remote = if touched.is_empty() {
            None
        } else {
            self.remote_intent(touched, caps)
        };

        Applied {
            forward: Patch {
                id: ChangeId::generate(),
                changes: forward,
            },
            inverse: Patch {
                id: ChangeId::generate(),
                changes: inverse,
            },
            remote,
        }
    }

    /// What, if anything, the server must be told — given what it is capable of.
    ///
    /// `None` means genuinely local: the capability says so, or the op has no server
    /// representation at all. This is the only consumer of `caps`, which is the whole reason
    /// `apply` takes it.
    fn remote_intent(&self, messages: Vec<MessageId>, caps: &AccountCaps) -> Option<RemoteIntent> {
        // Takes `messages` rather than capturing it: each arm consumes the vector once, and
        // a capturing closure would move it before the match could choose an arm.
        let mailbox = |messages: Vec<MessageId>, role: MailboxRole| match caps.archive {
            // Nothing to say: the server has no notion of where this message is filed.
            ArchiveMeans::LocalOnly => None,
            ArchiveMeans::DropInbox | ArchiveMeans::MoveToFolder(_) => {
                Some(RemoteIntent::SetMailbox { messages, role })
            }
        };
        match self {
            Op::Archive => mailbox(messages, MailboxRole::Archive),
            Op::Trash => mailbox(messages, MailboxRole::Trash),
            Op::Spam => mailbox(messages, MailboxRole::Spam),
            Op::Restore => mailbox(messages, MailboxRole::Inbox),
            Op::SetRead(state) => Some(RemoteIntent::SetFlags {
                messages,
                read: Some(*state),
                star: None,
            }),
            Op::SetStar(star) => Some(RemoteIntent::SetFlags {
                messages,
                read: None,
                star: Some(*star),
            }),
            Op::Label(label, membership) => match caps.labels {
                ServerLabels::LocalOnly => None,
                ServerLabels::Supported => {
                    let (add, remove) = match membership {
                        Membership::In => (vec![*label], Vec::new()),
                        Membership::Out => (Vec::new(), vec![*label]),
                    };
                    Some(RemoteIntent::SetLabels {
                        messages,
                        add,
                        remove,
                    })
                }
            },
            // Where a message is filed decides both halves, so it follows archiving: nothing to
            // say to a server that has no notion of where a message is.
            Op::File(label) => match caps.archive {
                ArchiveMeans::LocalOnly => None,
                ArchiveMeans::DropInbox | ArchiveMeans::MoveToFolder(_) => {
                    Some(RemoteIntent::File {
                        messages,
                        label: *label,
                    })
                }
            },
            // Neither has any server representation: they are this app's own state.
            Op::SetSnooze(_) | Op::SetPin(_) => None,
        }
    }

    /// This operation without its payload.
    pub fn kind(&self) -> OpKind {
        match self {
            Op::Archive => OpKind::Archive,
            Op::Trash => OpKind::Trash,
            Op::Restore => OpKind::Restore,
            Op::Spam => OpKind::Spam,
            Op::SetRead(ReadState::Read) => OpKind::MarkRead,
            Op::SetRead(ReadState::Unread) => OpKind::MarkUnread,
            Op::SetStar(Star::Starred) => OpKind::Star,
            Op::SetStar(Star::Unstarred) => OpKind::Unstar,
            Op::Label(_, Membership::In) => OpKind::AddLabel,
            Op::Label(_, Membership::Out) => OpKind::RemoveLabel,
            Op::SetSnooze(_) => OpKind::Snooze,
            Op::SetPin(_) => OpKind::Pin,
            // Filing is archiving into a named place: the row leaves the inbox the same way.
            Op::File(_) => OpKind::Archive,
        }
    }
}

/// The messages this target names, in the order `messages` holds them.
///
/// [`Target::Threads`] fans out: naming a thread names every message in it, which is why
/// `apply` is given the messages at all. [`Target::Messages`] acts on exactly what it names,
/// so a target may select a subset of a thread, or -- when it names another thread entirely --
/// none of it.
fn select<'m>(target: &Target, messages: &'m [Message]) -> Vec<&'m Message> {
    match target {
        Target::Threads(ids) => messages
            .iter()
            .filter(|m| ids.contains(&m.thread))
            .collect(),
        Target::Messages(ids) => messages.iter().filter(|m| ids.contains(&m.id)).collect(),
    }
}

/// File every selected message under `role`, remembering where each one came *from*.
///
/// The inverse is per message, not per op: a thread whose messages sit in `Inbox` and `Sent`
/// archives to one role but restores to two. Collapsing that to a single "put it back in the
/// inbox" silently moves the user's sent mail.
fn move_into(selected: &[&Message], role: MailboxRole) -> (Vec<Change>, Vec<Change>) {
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    for m in selected {
        // A message already filed there is untouched, so undoing does not move it either.
        if m.mailbox != role {
            forward.push(Change::MessageMailbox(m.id, role));
            inverse.push(Change::MessageMailbox(m.id, m.mailbox));
        }
    }
    (forward, inverse)
}

/// File every selected message into the folder `label` names: out to Archive, and into the label.
///
/// Both halves are undone per message, and each only where it changed something — a message
/// already archived keeps its place on undo, and one that already had the label keeps it.
fn file_into(selected: &[&Message], label: LabelId) -> (Vec<Change>, Vec<Change>) {
    let (mut forward, mut inverse) = move_into(selected, MailboxRole::Archive);
    let (labelled, unlabelled) = set_label(selected, label, Membership::In);
    forward.extend(labelled);
    inverse.extend(unlabelled);
    (forward, inverse)
}

/// Bring filed-away messages back to the inbox.
///
/// `Trash`, `Archive` and `Spam` are the three roles a message is filed *into*; restore is the
/// only way back out of any of them, `Spam` included. `Sent` and `Drafts` are left alone: they
/// record where a message came from rather than where the user put it, and moving them to the
/// inbox would lose that, irreversibly from the server's point of view.
fn restore(selected: &[&Message]) -> (Vec<Change>, Vec<Change>) {
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    for m in selected {
        if matches!(
            m.mailbox,
            MailboxRole::Trash | MailboxRole::Archive | MailboxRole::Spam
        ) {
            forward.push(Change::MessageMailbox(m.id, MailboxRole::Inbox));
            inverse.push(Change::MessageMailbox(m.id, m.mailbox));
        }
    }
    (forward, inverse)
}

/// Set every selected message to `state`, remembering each message's own prior value, so that
/// undoing "mark all read" over a half-read thread restores the halves.
fn set_read(selected: &[&Message], state: ReadState) -> (Vec<Change>, Vec<Change>) {
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    for m in selected {
        if m.read != state {
            forward.push(Change::MessageRead(m.id, state));
            inverse.push(Change::MessageRead(m.id, m.read));
        }
    }
    (forward, inverse)
}

/// As [`set_read`], for the star flag.
fn set_star(selected: &[&Message], star: Star) -> (Vec<Change>, Vec<Change>) {
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    for m in selected {
        if m.star != star {
            forward.push(Change::MessageStar(m.id, star));
            inverse.push(Change::MessageStar(m.id, m.star));
        }
    }
    (forward, inverse)
}

/// Move `label` in or out of every selected message's label list.
///
/// A message that is already on the wanted side contributes nothing to `forward` **and
/// nothing to `inverse`**. This is the case a standalone `Op::invert()` cannot get right:
/// adding a label the user already applied is a no-op, and undoing it must not strip a label
/// they had before the op ran.
///
/// A message's label list is a *set*; the order of [`Message::labels`] carries no meaning, so
/// an undo restores membership rather than position.
fn set_label(
    selected: &[&Message],
    label: LabelId,
    membership: Membership,
) -> (Vec<Change>, Vec<Change>) {
    let mut forward = Vec::new();
    let mut inverse = Vec::new();
    for m in selected {
        let present = m.labels.contains(&label);
        let wanted = membership == Membership::In;
        if present != wanted {
            forward.push(Change::MessageLabel(m.id, label, membership));
            inverse.push(Change::MessageLabel(m.id, label, membership.flip()));
        }
    }
    (forward, inverse)
}
