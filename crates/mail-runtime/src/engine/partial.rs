//! An operation refused after the server had done part of it (FINDINGS F159).
//!
//! An operation on messages in several IMAP mailboxes goes as one part per mailbox
//! ([`super::split::per_mailbox`], FINDINGS F154). When an early part is carried out and a later
//! one refused for good, the whole entry used to be undone here, and the first part's messages
//! were shown back where the server no longer had them. Now only the messages the server did not
//! act on are put back ([`mail_store::Settle::InPart`]), and the person is told which part went.
//!
//! Which messages those are is read from the parts as they were sent, against who each address
//! was before anything went: a move gives its messages new addresses, and a refusal after it must
//! still tell whose the old ones were. A message some part acted on is left as it is, even if
//! another of its addresses was in the refused part: the server has the change for it somewhere,
//! and on Gmail, where one message is at two addresses, it has it for the message. The next sync
//! says what the server holds either way.

use crate::RuntimeError;
use mail_domain::{AccountId, MessageId, Patch, ProtoOp, RemoteRef};
use mail_store::{Store, StoreError};

/// How far a split operation got: the parts the server answered, in order, and the parts it did
/// not, the first of them the one that failed.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    pub(crate) answered: Vec<ProtoOp>,
    pub(crate) left: Vec<ProtoOp>,
}

/// The messages an undo would put back, each with every address it has now.
///
/// Taken before an operation split per mailbox is sent. A message already gone from this client
/// has none.
pub(crate) fn held(
    store: &dyn Store,
    undo: &Patch,
) -> Result<Vec<(MessageId, Vec<RemoteRef>)>, RuntimeError> {
    let mut out: Vec<(MessageId, Vec<RemoteRef>)> = Vec::new();
    for message in undo.changes.iter().filter_map(mail_store::message_of) {
        if out.iter().any(|(m, _)| *m == message) {
            continue;
        }
        let remotes = match store.remotes_of(message) {
            Ok(remotes) => remotes,
            Err(StoreError::NoMessage(_)) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        out.push((message, remotes));
    }
    Ok(out)
}

/// The addresses an operation on messages names; none for any other.
fn remotes(op: &ProtoOp) -> &[RemoteRef] {
    match op {
        ProtoOp::FetchHeaders { remotes }
        | ProtoOp::FetchBody { remotes }
        | ProtoOp::SetFlags { remotes, .. }
        | ProtoOp::SetMailbox { remotes, .. }
        | ProtoOp::SetLabels { remotes, .. }
        | ProtoOp::File { remotes, .. }
        | ProtoOp::AddKeyword { remotes, .. }
        | ProtoOp::Expunge { remotes }
        | ProtoOp::FetchStructure { remotes } => remotes,
        _ => &[],
    }
}

/// The IMAP mailbox a part acts in, for saying so.
fn mailbox_of(op: &ProtoOp) -> Option<&str> {
    remotes(op).iter().find_map(|remote| match remote {
        RemoteRef::Imap { mailbox, .. } => Some(mailbox.as_str()),
        _ => None,
    })
}

/// Whether two paths name one folder: exactly, or `INBOX` in any case (RFC 9051 §5.1).
fn same_folder(a: &str, b: &str) -> bool {
    a == b || (a.eq_ignore_ascii_case("INBOX") && b.eq_ignore_ascii_case("INBOX"))
}

/// Whether every one of `remotes`, of which there is at least one, is in `folder`.
fn all_in(remotes: &[RemoteRef], folder: &str) -> bool {
    !remotes.is_empty()
        && remotes.iter().all(|remote| match remote {
            RemoteRef::Imap { mailbox, .. } => same_folder(mailbox, folder),
            _ => false,
        })
}

/// Of `held`, the messages the server has this operation's change for once it stopped at
/// `progress`: named by a part it answered, or, for a move into `target`, already there, which a
/// part not reached would have left as it is (FINDINGS F156).
pub(crate) fn done(
    held: &[(MessageId, Vec<RemoteRef>)],
    progress: &Progress,
    target: Option<&str>,
) -> Vec<MessageId> {
    let answered: Vec<&RemoteRef> = progress.answered.iter().flat_map(remotes).collect();
    held.iter()
        .filter(|(_, at)| {
            at.iter().any(|remote| answered.contains(&remote))
                || target.is_some_and(|folder| all_in(at, folder))
        })
        .map(|(message, _)| *message)
        .collect()
}

/// Of the messages `undo` would put back, those already where a move into `target` puts them,
/// by the addresses they have now or by where a move filed them without saying where: an entry
/// given up before it was sent ([`mail_store::Dispatch::Lost`]) is not to take them out of it.
pub(crate) fn already_there(
    store: &dyn Store,
    account: AccountId,
    undo: &Patch,
    target: &str,
) -> Result<Vec<MessageId>, RuntimeError> {
    let mut out = Vec::new();
    for (message, at) in held(store, undo)? {
        let there = all_in(&at, target)
            || (at.is_empty()
                && store
                    .unplaced_into(account, message)?
                    .is_some_and(|folder| same_folder(&folder, target)));
        if there {
            out.push(message);
        }
    }
    Ok(out)
}

/// What the person is told of an operation refused after part of it was done: where the server
/// did it, where it refused it and why, and that only the rest was undone.
pub(crate) fn said(progress: &Progress, target: Option<&str>, why: &RuntimeError) -> String {
    let mut went: Vec<&str> = Vec::new();
    for part in &progress.answered {
        if let Some(mailbox) = mailbox_of(part)
            && !went.contains(&mailbox)
        {
            went.push(mailbox);
        }
    }
    let refused = progress
        .left
        .first()
        .and_then(mailbox_of)
        .unwrap_or("another mailbox");
    let went = match (went.is_empty(), target) {
        (false, _) => format!("in {}", went.join(", ")),
        (true, Some(folder)) => format!("for the messages already in {folder}"),
        (true, None) => "for some of its messages".to_owned(),
    };
    format!(
        "Done only in part: the server did this {went}, and refused it in {refused} ({why}). \
         What it did is kept here, and the rest has been undone."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::MailboxRole;

    fn imap(mailbox: &str, uid: u32) -> RemoteRef {
        RemoteRef::Imap {
            mailbox: mailbox.to_owned(),
            uidvalidity: 1,
            uid,
        }
    }

    fn part(remotes: Vec<RemoteRef>) -> ProtoOp {
        ProtoOp::SetMailbox {
            remotes,
            role: MailboxRole::Archive,
        }
    }

    #[test]
    fn a_message_is_done_when_a_part_the_server_answered_named_it() {
        let [a, b, c] = [
            MessageId::generate(),
            MessageId::generate(),
            MessageId::generate(),
        ];
        let held = vec![
            (a, vec![imap("INBOX", 1)]),
            // At two addresses, one in the part that went: the server has it somewhere.
            (b, vec![imap("INBOX", 2), imap("Work", 7)]),
            (c, vec![imap("Work", 8)]),
        ];
        let progress = Progress {
            answered: vec![part(vec![imap("INBOX", 1), imap("INBOX", 2)])],
            left: vec![part(vec![imap("Work", 7), imap("Work", 8)])],
        };
        assert_eq!(done(&held, &progress, None), vec![a, b]);
    }

    #[test]
    fn a_message_a_move_finds_already_in_its_target_is_done_though_its_part_was_not_reached() {
        let [a, b] = [MessageId::generate(), MessageId::generate()];
        let held = vec![(a, vec![imap("Work", 3)]), (b, vec![imap("Archive", 1)])];
        let progress = Progress {
            answered: Vec::new(),
            left: vec![part(vec![imap("Work", 3)]), part(vec![imap("Archive", 1)])],
        };
        assert_eq!(done(&held, &progress, Some("Archive")), vec![b]);
        assert!(done(&held, &progress, None).is_empty());
    }

    #[test]
    fn what_is_said_names_where_it_went_and_where_it_was_refused() {
        let progress = Progress {
            answered: vec![part(vec![imap("INBOX", 1)])],
            left: vec![part(vec![imap("Work", 3)])],
        };
        let why = RuntimeError::UnsupportedIo("no".to_owned());
        let text = said(&progress, Some("Archive"), &why);
        assert!(text.contains("did this in INBOX"), "{text}");
        assert!(text.contains("refused it in Work"), "{text}");
    }
}
