//! Messages a search of the server brought here, and messages by server address, in the
//! in-memory store. Kept in step with `sqlite/found.rs`, which `tests/found.rs` holds it to.

use super::{Inner, same_remote};
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, MessageId, RemoteRef, ThreadId};

impl Inner {
    pub(super) fn mark_found(
        &mut self,
        account: AccountId,
        messages: &[MessageId],
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        if let Some(missing) = messages
            .iter()
            .find(|id| self.messages.get(id).is_none_or(|m| m.account != account))
        {
            return Err(StoreError::NoMessage(*missing));
        }
        for id in messages {
            // The first search to bring it stands, as SQLite's `INSERT OR IGNORE`.
            self.found.entry(*id).or_insert((account, now));
        }
        Ok(())
    }

    pub(super) fn found_in(&self, threads: &[ThreadId]) -> Vec<ThreadId> {
        let mut out: Vec<ThreadId> = Vec::new();
        for thread in threads {
            if out.contains(thread) {
                continue;
            }
            let found = self.found.keys().any(|id| {
                self.messages
                    .get(id)
                    .is_some_and(|message| message.thread == *thread)
            });
            if found {
                out.push(*thread);
            }
        }
        out
    }

    pub(super) fn held_at(
        &self,
        account: AccountId,
        remotes: &[RemoteRef],
    ) -> Vec<(RemoteRef, MessageId)> {
        remotes
            .iter()
            .filter_map(|remote| {
                self.remotes
                    .iter()
                    .find(|row| same_remote(row, account, remote))
                    .map(|row| (remote.clone(), row.message))
            })
            .collect()
    }
}
