use crate::undo::Undo;
use crate::view::op_for;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// The composer pane.
///
/// Every field writes straight back into `Shell.composing`, and every button goes through
/// `crate::compose`, which is the same module the CLI calls. Two code paths for "send this
/// draft" is how a window and a command start disagreeing about what a draft is.
/// What opening a composer on this action means.
///
/// Three of the row's buttons open a composer rather than performing an operation. A forward is
/// not a reply with a different audience — it carries the message rather than answering it, and
/// it starts with no recipients — so it is a variant here rather than a third `ReplyScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Composes {
    Reply(ReplyScope),
    Forward,
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
/// Which message that answers is [`crate::view::reply_target`]'s decision, not this function's.
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
        Some((_, id)) => *id,
        None => crate::compose::account_for(store, None)?,
    };
    // No recipients and no subject: there is no original to take either from, and a guess is
    // something the sender has to notice and undo. Saved anyway, so closing the window keeps it.
    crate::compose::draft_new(store, account, &[], "", "", chrono::Utc::now())
}

pub(super) fn start_composing(
    store: &SqliteStore,
    thread: ThreadId,
    what: Composes,
) -> Result<Draft, String> {
    match what {
        Composes::Reply(scope) => start_reply(store, thread, scope),
        Composes::Forward => {
            let loaded = store.thread(thread).map_err(|e| e.to_string())?;
            let messages: Vec<Message> = loaded
                .messages
                .iter()
                .filter_map(|id| store.message(*id).ok())
                .collect();
            let target = crate::view::reply_target(&messages)
                .ok_or_else(|| "that conversation has no message to forward".to_owned())?;
            // No recipients: a forward has none of its own and the composer is where the user
            // names them. The draft is saved regardless, so closing the window does not lose it.
            crate::compose::draft_forward(store, target.id, &[], "", chrono::Utc::now())
        }
    }
}

fn start_reply(store: &SqliteStore, thread: ThreadId, scope: ReplyScope) -> Result<Draft, String> {
    let loaded = store.thread(thread).map_err(|e| e.to_string())?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let target = crate::view::reply_target(&messages)
        .ok_or_else(|| "that conversation has no messages".to_owned())?;
    crate::compose::draft_reply(store, target.id, scope, "", chrono::Utc::now())
}

/// Apply a hover action, returning whether anything changed.
///
/// A free function rather than a closure so it can be called from several handlers, and so the
/// store it needs is an argument rather than a capture.
/// Put a label on a conversation, or take it off.
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

pub(super) fn apply_op(store: &SqliteStore, thread: ThreadId, kind: OpKind) -> bool {
    resolve(store, thread, kind).is_some_and(|op| apply(store, thread, op))
}

/// The operation a button means for this conversation now.
///
/// `Pin` needs a payload `op_for` cannot supply — the direction comes from the conversation's
/// current state and the rank from the clock — so it is resolved here, where both are in
/// reach. Label and snooze open a menu rather than acting, and resolve to nothing.
pub(super) fn resolve(store: &SqliteStore, thread: ThreadId, kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Pin => {
            let loaded = store.thread(thread).ok()?;
            Some(crate::view::pin_op(&loaded.summary, chrono::Utc::now()))
        }
        other => op_for(other),
    }
}

fn apply(store: &SqliteStore, thread: ThreadId, op: Op) -> bool {
    perform(store, thread, op).is_some()
}

/// Take back one operation: its inverse, written like any other patch, and the reverse queued
/// for the server when the operation had told it anything.
pub(super) fn take_back(store: &SqliteStore, entry: &Undo) -> bool {
    if store.apply(entry.account, &entry.inverse).is_err() {
        return false;
    }
    if let Some(reverse) = entry
        .remote
        .as_ref()
        .and_then(|remote| crate::undo::reverse_intent(remote, &entry.inverse))
    {
        let _ = store.enqueue(entry.account, reverse, &entry.forward, chrono::Utc::now());
    }
    true
}

/// Apply one resolved operation: locally, and to the server when it has a server half.
///
/// Returns what it takes to undo it, which the window keeps for the toast and Ctrl Z.
pub(super) fn perform(store: &SqliteStore, thread: ThreadId, op: Op) -> Option<Undo> {
    let loaded = store.thread(thread).ok()?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let account = messages.first().map(|m| m.account)?;
    // What the server actually supports, which is the only thing that decides whether an
    // operation gets a `RemoteIntent` at all — `Op::remote_intent` is the sole consumer of
    // `caps`, and under `ArchiveMeans::LocalOnly` it returns `None` for Archive and Trash.
    // This used to be a hardcoded struct of safe defaults, so archiving in the window changed
    // nothing on the server and the conversation came back on the next full sync. See F139.
    //
    // The defaults below are still the answer before the first sync, when nothing is known.
    let caps = crate::sync::caps_of(store, account).unwrap_or(AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: chrono::Utc::now(),
    });
    let applied = op.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &caps,
        chrono::Utc::now(),
    );
    store.apply(account, &applied.forward).ok()?;
    // And the server's half. `Applied.remote` was computed and dropped on the floor, so every
    // operation the window performed was local and stayed local: a conversation archived here
    // was still in the inbox on the phone, and mail read here was still bold everywhere else.
    // The undo goes with it, because a submission that fails has to put back what it changed.
    //
    // A failure to enqueue is not a failure of the operation: the local change is real and the
    // user can see it. It surfaces where every other stalled submission does, in the outbox.
    if let Some(intent) = applied.remote.clone() {
        let _ = store.enqueue(account, intent, &applied.inverse, chrono::Utc::now());
    }
    Some(Undo {
        said: crate::undo::said(&op, &chrono::Local),
        thread,
        account,
        forward: applied.forward,
        inverse: applied.inverse,
        remote: applied.remote,
    })
}

#[cfg(test)]
mod tests {
    use super::apply_op;
    use crate::ui::fixtures::{ACCOUNT, gmail_caps, inbox_query, realistic};
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};

    /// What the server is told when the window acts — F139.
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

            let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
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

            let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
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
            store
                .connection()
                .execute(
                    "UPDATE account_caps SET caps = ?2 WHERE account = ?1",
                    rusqlite::params![
                        ACCOUNT.to_string(),
                        serde_json::to_string(&local_only).unwrap()
                    ],
                )
                .unwrap();
            let thread = first_thread(&store);

            assert!(apply_op(&store, thread, OpKind::Archive));
            let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
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
            let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
            assert!(
                queued.is_empty(),
                "{:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }
    }
}
