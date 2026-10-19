//! The values a test starts from, so it states only what it is about.
//!
//! A test of a list row or a filter needs a whole [`ThreadSummary`], and nearly every field of it
//! is beside the point. Each test used to spell all of them out; now it writes
//! `ThreadSummary { subject, read, ..summary() }`. The defaults are an unread, unstarred
//! conversation of one message in the Inbox, with every thread-level setting off.

use crate::account::Identity;
use crate::address::Address;
use crate::id::{IdentityId, ThreadId, new_account_id};
use crate::message::ThreadSummary;
use crate::state::{
    Attachments, FollowUp, IsDefault, MailboxRole, MailboxSet, Mute, Pin, ReadState, Snooze, Star,
};
use chrono::{TimeZone, Utc};

/// An unread, unstarred one-message conversation in the Inbox, from `a@example.test`, on an
/// account of its own.
pub fn summary() -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::generate(),
        account: new_account_id(),
        subject: "s".into(),
        snippet: String::new(),
        from: Address {
            name: None,
            email: "a@example.test".into(),
        },
        participants: vec![],
        recipients: vec![],
        last_date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        message_count: 1,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailboxes: MailboxSet::only(MailboxRole::Inbox),
        labels: vec![],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
        mute: Mute::Unmuted,
        follow_up: FollowUp::Inactive,
    }
}

/// The default identity of an account of its own, `me@example.test`, with no signature.
pub fn identity() -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: new_account_id(),
        from: Address {
            name: None,
            email: "me@example.test".into(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}
