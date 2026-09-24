//! One operation per IMAP mailbox (FINDINGS F154).
//!
//! An IMAP UID means something only in the mailbox it came from, and a `UID STORE` or
//! `UID MOVE` acts on the one mailbox selected. A message is often at two addresses (Gmail's
//! INBOX and Sent Mail; a copy in a folder), and a conversation spans folders, so an operation's
//! addresses name several mailboxes. The backend runs an operation in the first mailbox its
//! addresses name, so sent whole, every UID was read in that mailbox, where it may name another
//! message.
//!
//! Here rather than as a method on `ProtoOp`: `mail-domain` is the frozen interface
//! (CONVENTIONS §1), and the engine is the only caller.

use mail_domain::{ProtoOp, RemoteRef};

/// The addresses `op` acts on, when it acts on messages.
fn remotes_mut(op: &mut ProtoOp) -> Option<&mut Vec<RemoteRef>> {
    match op {
        ProtoOp::FetchHeaders { remotes }
        | ProtoOp::FetchBody { remotes }
        | ProtoOp::SetFlags { remotes, .. }
        | ProtoOp::SetMailbox { remotes, .. }
        | ProtoOp::SetLabels { remotes, .. }
        | ProtoOp::File { remotes, .. }
        | ProtoOp::AddKeyword { remotes, .. }
        | ProtoOp::Expunge { remotes }
        | ProtoOp::FetchStructure { remotes } => Some(remotes),
        _ => None,
    }
}

/// `op` as one per IMAP mailbox its messages are in, in the order each mailbox first appears;
/// anything else, or one mailbox's worth, as it is.
pub(crate) fn per_mailbox(mut op: ProtoOp) -> Vec<ProtoOp> {
    let Some(remotes) = remotes_mut(&mut op) else {
        return vec![op];
    };
    let mut groups: Vec<(Option<String>, Vec<RemoteRef>)> = Vec::new();
    for remote in std::mem::take(remotes) {
        // Only IMAP selects a mailbox; every other address is complete on its own.
        let key = match &remote {
            RemoteRef::Imap { mailbox, .. } => Some(mailbox.clone()),
            _ => None,
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, group)) => group.push(remote),
            None => groups.push((key, vec![remote])),
        }
    }
    if groups.len() <= 1 {
        if let (Some(remotes), Some((_, group))) = (remotes_mut(&mut op), groups.pop()) {
            *remotes = group;
        }
        return vec![op];
    }
    groups
        .into_iter()
        .map(|(_, group)| {
            let mut one = op.clone();
            if let Some(remotes) = remotes_mut(&mut one) {
                *remotes = group;
            }
            one
        })
        .collect()
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

    #[test]
    fn an_operation_over_two_mailboxes_becomes_one_per_mailbox_in_order() {
        let op = ProtoOp::SetMailbox {
            remotes: vec![imap("INBOX", 5), imap("Archive", 9), imap("INBOX", 6)],
            role: MailboxRole::Trash,
        };
        assert_eq!(
            per_mailbox(op),
            vec![
                ProtoOp::SetMailbox {
                    remotes: vec![imap("INBOX", 5), imap("INBOX", 6)],
                    role: MailboxRole::Trash,
                },
                ProtoOp::SetMailbox {
                    remotes: vec![imap("Archive", 9)],
                    role: MailboxRole::Trash,
                },
            ]
        );
    }

    #[test]
    fn one_mailbox_or_no_messages_is_left_as_it_is() {
        let one = ProtoOp::SetFlags {
            remotes: vec![imap("INBOX", 5), imap("INBOX", 6)],
            read: None,
            star: None,
        };
        assert_eq!(per_mailbox(one.clone()), vec![one]);
        assert_eq!(per_mailbox(ProtoOp::FetchCaps), vec![ProtoOp::FetchCaps]);
        let graph = ProtoOp::File {
            remotes: vec![
                RemoteRef::Graph {
                    mailbox: "INBOX".to_owned(),
                    id: "a".to_owned(),
                },
                RemoteRef::Graph {
                    mailbox: "Work".to_owned(),
                    id: "b".to_owned(),
                },
            ],
            folder: "Done".to_owned(),
        };
        assert_eq!(per_mailbox(graph.clone()), vec![graph]);
    }
}
