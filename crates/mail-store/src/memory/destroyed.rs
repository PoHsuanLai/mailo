//! Addresses of messages deleted forever, in the in-memory store. Kept in step with
//! `sqlite/destroyed.rs`, which says why they are kept; the parity tests in `tests/destroy.rs`
//! hold the two to each other.

use super::{Inner, RemoteRow, remote_parts, row_to_remote, same_remote};
use crate::StoreError;
use crate::dispatch::Answered;
use mail_domain::{AccountId, MailboxRef, MessageId, OutboxId, RemoteRef};

/// One kept address: where the destroyed message was, and the entry deleting it there.
#[derive(Debug, Clone)]
pub(super) struct DestroyedRow {
    pub(super) at: RemoteRow,
    /// `None` once the server confirmed the deletion.
    pub(super) outbox: Option<OutboxId>,
}

impl Inner {
    pub(super) fn keep_destroyed(
        &mut self,
        account: AccountId,
        message: MessageId,
        remote: &RemoteRef,
        outbox: OutboxId,
    ) {
        self.destroyed
            .retain(|row| !same_remote(&row.at, account, remote));
        let (mailbox, uidvalidity, uid, uidl) = remote_parts(remote);
        self.destroyed.push(DestroyedRow {
            at: RemoteRow {
                account,
                mailbox,
                uidvalidity,
                uid,
                uidl,
                message,
            },
            outbox: Some(outbox),
        });
    }

    pub(super) fn destroyed_in(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, StoreError> {
        self.destroyed
            .iter()
            .filter(|row| row.at.account == mailbox.account && row.at.mailbox == mailbox.path)
            .map(|row| row_to_remote(&row.at))
            .collect()
    }

    pub(super) fn destroyed_of(
        &self,
        account: AccountId,
        message: MessageId,
    ) -> Result<Vec<RemoteRef>, StoreError> {
        self.destroyed
            .iter()
            .filter(|row| row.at.account == account && row.at.message == message)
            .map(|row| row_to_remote(&row.at))
            .collect()
    }

    pub(super) fn forget_destroyed(&mut self, account: AccountId, remote: &RemoteRef) {
        self.destroyed
            .retain(|row| !same_remote(&row.at, account, remote));
    }

    pub(super) fn forget_destroyed_in(&mut self, account: AccountId, path: &str) {
        self.destroyed
            .retain(|row| !(row.at.account == account && row.at.mailbox == path));
    }

    pub(super) fn remap_destroyed(&mut self, account: AccountId, from: &RemoteRef, to: &RemoteRef) {
        if from == to
            || !self
                .destroyed
                .iter()
                .any(|row| same_remote(&row.at, account, from))
        {
            return;
        }
        // As `UPDATE OR REPLACE`: a row already kept at `to` gives way.
        self.destroyed
            .retain(|row| !same_remote(&row.at, account, to));
        let Some(at) = self
            .destroyed
            .iter()
            .position(|row| same_remote(&row.at, account, from))
        else {
            return;
        };
        let (mailbox, uidvalidity, uid, uidl) = remote_parts(to);
        let row = &mut self.destroyed[at].at;
        row.mailbox = mailbox;
        row.uidvalidity = uidvalidity;
        row.uid = uid;
        row.uidl = uidl;
    }

    pub(super) fn settle_destroyed(&mut self, outbox: OutboxId, answered: Answered) {
        match answered {
            Answered::Done => {
                for row in self
                    .destroyed
                    .iter_mut()
                    .filter(|row| row.outbox == Some(outbox))
                {
                    row.outbox = None;
                }
            }
            Answered::Refused => self.destroyed.retain(|row| row.outbox != Some(outbox)),
        }
    }

    pub(super) fn rename_destroyed(
        &mut self,
        account: AccountId,
        rename: &dyn Fn(&str) -> Option<String>,
    ) {
        for row in self
            .destroyed
            .iter_mut()
            .filter(|row| row.at.account == account)
        {
            if let Some(path) = rename(&row.at.mailbox) {
                row.at.mailbox = path;
            }
        }
    }
}
