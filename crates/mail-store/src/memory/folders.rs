//! Mailboxes in the in-memory store. Kept in step with `sqlite/folders.rs`, which the parity
//! tests in `tests/folders.rs` hold it to.

use super::Inner;
use mail_domain::folder::{layered, renamed};
use mail_domain::{
    AccountId, Folder, FolderContents, FolderWork, LabelId, LabelOrigin, MailboxRef, MessageId,
    ProtoOp,
};

impl Inner {
    pub(super) fn folders_of(&self, account: AccountId) -> Vec<Folder> {
        // Keyed by `(account, path)`, so this is already in path order.
        self.folders
            .iter()
            .filter(|((a, _), _)| *a == account)
            .map(|(_, f)| f.clone())
            .collect()
    }

    pub(super) fn put_folders(&mut self, account: AccountId, listed: Vec<Folder>) {
        let pending: Vec<FolderWork> = self
            .outbox
            .values()
            .filter(|row| row.account == account)
            .filter_map(|row| match &row.op {
                ProtoOp::Folder(work) => Some(work.clone()),
                _ => None,
            })
            .collect();
        self.accounts.insert(account);
        self.folders.retain(|(a, _), _| *a != account);
        for folder in layered(account, listed, &pending) {
            self.folders
                .insert((folder.account, folder.path.clone()), folder);
        }
    }

    pub(super) fn upsert_folder(&mut self, folder: &Folder) {
        self.accounts.insert(folder.account);
        self.folders
            .insert((folder.account, folder.path.clone()), folder.clone());
    }

    pub(super) fn remove_folder(&mut self, mailbox: &MailboxRef) {
        self.folders
            .remove(&(mailbox.account, mailbox.path.clone()));
    }

    /// As `SqliteStore::rename_folder`: the folders, the server addresses, the cursors and the
    /// server's labels.
    pub(super) fn rename_folder(&mut self, from: &MailboxRef, to: &str, delimiter: Option<char>) {
        let account = from.account;
        let rename = |path: &str| renamed(path, &from.path, to, delimiter);

        let moved: Vec<((AccountId, String), Folder)> = self
            .folders
            .iter()
            .filter(|((a, _), _)| *a == account)
            .filter_map(|(key, folder)| {
                let path = rename(&folder.path)?;
                let mut folder = folder.clone();
                folder.path = path;
                Some((key.clone(), folder))
            })
            .collect();
        for (old, folder) in moved {
            self.folders.remove(&old);
            self.folders
                .insert((folder.account, folder.path.clone()), folder);
        }

        for row in self.remotes.iter_mut().filter(|r| r.account == account) {
            if let Some(path) = rename(&row.mailbox) {
                row.mailbox = path;
            }
        }

        let cursors: Vec<(AccountId, String)> = self
            .sync
            .keys()
            .filter(|(a, path)| *a == account && rename(path).is_some())
            .cloned()
            .collect();
        for key in cursors {
            if let (Some(cursor), Some(path)) = (self.sync.remove(&key), rename(&key.1)) {
                self.sync.insert((account, path), cursor);
            }
        }

        for label in self
            .labels
            .values_mut()
            .filter(|l| l.account == account && l.origin == LabelOrigin::Provider)
        {
            if let Some(name) = rename(&label.name) {
                label.name = name;
            }
        }
    }

    pub(super) fn remove_label(&mut self, label: LabelId) {
        self.labels.remove(&label);
        for message in self.messages.values_mut() {
            message.labels.retain(|l| *l != label);
        }
    }

    pub(super) fn contents(&self, mailbox: &MailboxRef) -> FolderContents {
        let mut mapped: Vec<MessageId> = self
            .remotes
            .iter()
            .filter(|r| r.account == mailbox.account && r.mailbox == mailbox.path)
            .map(|r| r.message)
            .collect();
        mapped.sort();
        mapped.dedup();

        let labels: Vec<LabelId> = self
            .labels
            .values()
            .filter(|l| {
                l.account == mailbox.account
                    && l.name == mailbox.path
                    && l.origin == LabelOrigin::Provider
            })
            .map(|l| l.id)
            .collect();
        let mut labelled: Vec<MessageId> = self
            .messages
            .values()
            .filter(|m| m.labels.iter().any(|l| labels.contains(l)))
            .map(|m| m.id)
            .collect();
        labelled.sort();
        FolderContents { mapped, labelled }
    }

    /// As `SqliteStore::forget_mailbox`.
    pub(super) fn forget_mailbox(&mut self, account: AccountId, path: &str) {
        let held = self.contents(&MailboxRef {
            account,
            path: path.to_owned(),
        });
        self.remotes
            .retain(|r| !(r.account == account && r.mailbox == path));
        self.sync.remove(&(account, path.to_owned()));
        for message in held.mapped {
            if !self.remotes.iter().any(|r| r.message == message) {
                self.delete_message(message);
            }
        }
    }
}
