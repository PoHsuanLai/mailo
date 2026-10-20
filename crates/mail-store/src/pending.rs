//! The local changes a [`RemoteIntent`] stands for, shared by both stores.

use mail_domain::{Change, Membership, MessageId, RemoteIntent};

/// The local changes this intent corresponds to, per message.
///
/// Derived from the intent rather than taken from the caller's forward patch, because the
/// intent is exactly "what the server is being asked to do" — which is precisely what has
/// to be re-layered on top of server truth until the server agrees.
pub(crate) fn pending_of(intent: &RemoteIntent) -> Vec<(MessageId, Vec<Change>)> {
    match intent {
        RemoteIntent::SetFlags {
            messages,
            read,
            star,
        } => messages
            .iter()
            .map(|id| {
                let mut changes = Vec::new();
                if let Some(read) = read {
                    changes.push(Change::MessageRead(*id, *read));
                }
                if let Some(star) = star {
                    changes.push(Change::MessageStar(*id, *star));
                }
                (*id, changes)
            })
            .collect(),
        RemoteIntent::SetMailbox { messages, role } => messages
            .iter()
            .map(|id| (*id, vec![Change::MessageMailbox(*id, *role)]))
            .collect(),
        RemoteIntent::SetLabels {
            messages,
            add,
            remove,
        } => messages
            .iter()
            .map(|id| {
                let mut changes = Vec::new();
                for label in add {
                    changes.push(Change::MessageLabel(*id, *label, Membership::In));
                }
                for label in remove {
                    changes.push(Change::MessageLabel(*id, *label, Membership::Out));
                }
                (*id, changes)
            })
            .collect(),
        RemoteIntent::File { messages, label } => messages
            .iter()
            .map(|id| {
                (
                    *id,
                    vec![
                        Change::MessageMailbox(*id, mail_domain::MailboxRole::Archive),
                        Change::MessageLabel(*id, *label, Membership::In),
                    ],
                )
            })
            .collect(),
        // A submission re-layers nothing: it has no existing message to be overwritten by the
        // next ingest. Matches SqliteStore.
        // A keyword has no local mirror on the message to re-layer: the answer it records is
        // kept by the store beside the message, written when the user answered.
        RemoteIntent::AddKeyword { .. } => Vec::new(),
        // As SqliteStore: a destroyed message has nothing to re-layer.
        RemoteIntent::Destroy { .. } => Vec::new(),
        RemoteIntent::Send { .. } | RemoteIntent::Folder(_) | RemoteIntent::Append { .. } => {
            Vec::new()
        }
    }
}
