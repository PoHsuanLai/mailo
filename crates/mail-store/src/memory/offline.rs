//! What of an account is held here, in the in-memory store. Kept in step with
//! `sqlite/offline.rs`, which `tests/offline.rs` holds it to.

use super::Inner;
use crate::offline::is_remote;
use crate::{Offline, RemotePart};
use mail_domain::{MailboxRef, PartContent};
use porter_core::AccountId;
use std::cmp::Reverse;

impl Inner {
    pub(super) fn remote_parts_in(&self, mailbox: &MailboxRef, limit: u32) -> Vec<RemotePart> {
        let mut parts: Vec<(RemotePart, chrono::DateTime<chrono::Utc>)> = self
            .messages
            .values()
            .filter(|m| m.account == mailbox.account)
            .filter(|m| {
                self.remotes.iter().any(|row| {
                    row.message == m.id
                        && row.account == mailbox.account
                        && row.mailbox == mailbox.path
                })
            })
            .flat_map(|m| {
                m.attachments
                    .iter()
                    .filter(|a| is_remote(a))
                    .filter_map(move |a| match &a.content {
                        PartContent::Remote { section } => Some((
                            RemotePart {
                                message: m.id,
                                section: section.clone(),
                                size: a.size,
                            },
                            m.date,
                        )),
                        PartContent::Held(_) => None,
                    })
            })
            .collect();
        // SQLite's `ORDER BY size, date DESC, id DESC, section`: ids compare as their text.
        parts.sort_by(|(a, a_date), (b, b_date)| {
            (
                a.size,
                Reverse(*a_date),
                Reverse(a.message.to_string()),
                &a.section,
            )
                .cmp(&(
                    b.size,
                    Reverse(*b_date),
                    Reverse(b.message.to_string()),
                    &b.section,
                ))
        });
        parts
            .into_iter()
            .map(|(part, _)| part)
            .take(limit as usize)
            .collect()
    }

    pub(super) fn offline(&self, account: AccountId) -> Offline {
        let mut offline = Offline::default();
        for message in self.messages.values().filter(|m| m.account == account) {
            offline.add(message);
        }
        offline
    }
}
