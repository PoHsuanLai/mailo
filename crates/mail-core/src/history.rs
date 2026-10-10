//! History: every conversation the person opened, newest first.
//!
//! Where Today is a Space's few recent tabs that expire, History is the whole record, kept in the
//! store for [`KEPT`] and shared by every Space; a Space only narrows what is listed, to its
//! accounts, as it narrows any place. An open is recorded by [`record`], which the window calls
//! when a conversation is shown, and the store forgets the old ones in the same write.

use crate::error::Logged;
use crate::follow_up::in_scope;
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::{Filter, ThreadId, ThreadSummary};
use mail_store::{SqliteStore, Store};

/// How long a conversation stays in History after it was last opened.
pub const KEPT: TimeDelta = mail_store::OPENED_KEPT;

/// Note that `thread` was opened at `now`. A store that cannot write it is logged and the open
/// goes ahead: History is a convenience, and failing to keep it must not stop anyone reading.
pub fn record(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) {
    store
        .record_opened(thread, now)
        .or_log_default("the open could not be added to History");
}

/// Every conversation opened, newest first, each once, narrowed to `scope`: the History place.
pub fn listed(
    store: &SqliteStore,
    scope: Option<&Filter>,
    now: DateTime<Utc>,
) -> Vec<ThreadSummary> {
    store
        .opened()
        .or_log_default("History could not be read")
        .into_iter()
        .filter(|summary| in_scope(summary, scope, now))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::id::account_id_from_uuid;
    use mail_domain::{
        Address, Body, Change, ChangeId, MailboxRole, Message, MessageId, MessageKey, Patch,
        ReadState, Star,
    };
    use porter_core::AccountId;

    fn account(n: u128) -> AccountId {
        account_id_from_uuid(uuid::Uuid::from_u128(0xa0 + n))
    }

    fn thread(n: u128) -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
    }

    fn day(n: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + TimeDelta::days(n)
    }

    /// One conversation of one message in `owner`'s account.
    fn put(store: &SqliteStore, n: u128, owner: AccountId) {
        let message = Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
            thread: thread(n),
            account: owner.clone(),
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: day(0),
            from: Address {
                name: None,
                email: "ada@example.test".to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: format!("subject {n}"),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Absent,
            attachments: vec![],
        };
        store
            .apply(
                owner,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .unwrap();
    }

    fn numbers(list: &[ThreadSummary]) -> Vec<u128> {
        list.iter()
            .map(|summary| summary.id.as_uuid().as_u128() - 0x7000)
            .collect()
    }

    #[test]
    fn history_lists_the_newest_open_first_and_narrows_to_a_spaces_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
        mail_store::testing::seed_account(&store, account(1), "one@example.test");
        mail_store::testing::seed_account(&store, account(2), "two@example.test");
        put(&store, 1, account(1));
        put(&store, 2, account(2));
        put(&store, 3, account(1));
        record(&store, thread(1), day(1));
        record(&store, thread(2), day(2));
        record(&store, thread(3), day(3));
        record(&store, thread(1), day(4));

        assert_eq!(numbers(&listed(&store, None, day(5))), [1, 3, 2]);
        let first = Filter::Account(account(1));
        assert_eq!(numbers(&listed(&store, Some(&first), day(5))), [1, 3]);
    }

    #[test]
    fn history_is_kept_ninety_days() {
        assert_eq!(KEPT, TimeDelta::days(90));
    }
}
