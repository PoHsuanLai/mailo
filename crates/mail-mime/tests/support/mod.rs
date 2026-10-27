//! What the tests of `mail-mime` share, declared once: the draft, sender and parent a built
//! message starts from, the walkers the block tests read a document with, and the throwaway
//! certificate authorities.
//!
//! The authorities stay in `bimi_support/` and `smime_support/`, where `mail-runtime`'s and
//! `mail-app`'s tests include them by path.

#[cfg(feature = "smime")]
#[path = "../bimi_support/mod.rs"]
pub mod bimi;
pub mod block;
#[cfg(feature = "smime")]
#[path = "../smime_support/mod.rs"]
pub mod smime;

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::new_account_id;
use mail_domain::{
    Address, BlobId, Body, Draft, DraftId, Identity, IdentityId, MailboxRole, Message, MessageId,
    MessageKey, ReadState, ReceiptRequest, SendState, Star, ThreadId,
};

pub fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// Sunday 2026-03-15, 12:00 UTC: when a draft was last edited and its parent arrived.
pub fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap()
}

/// The default identity, sending as `name <email>`.
pub fn sender(name: Option<&str>, email: &str) -> Identity {
    Identity {
        from: addr(name, email),
        ..mail_domain::testing::identity()
    }
}

/// A plain draft to Bea, "hello" and "hi", edited at [`at`].
pub fn draft() -> Draft {
    Draft {
        id: DraftId::generate(),
        account: new_account_id(),
        identity: IdentityId::generate(),
        to: vec![addr(Some("Bea"), "bea@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "hello".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "hi\r\n".to_owned(),
        html: None,
        attachments: Vec::new(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: mail_domain::OpenPgp::None,
        smime: mail_domain::Smime::None,
        priority: mail_domain::Priority::Normal,
        delivery: mail_domain::DeliveryReport::Unrequested,
        automatic: false,
        state: SendState::Editing,
        updated: at(),
    }
}

/// The message a reply answers: "Lunch" from Ada, with these `references` and Message-ID.
pub fn parent(references: Vec<String>, rfc_message_id: Option<&str>) -> Message {
    Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: new_account_id(),
        key: MessageKey::Rfc("parent@example.test".to_owned()),
        date: at(),
        from: addr(Some("Ada"), "ada@example.test"),
        reply_to: Vec::new(),
        to: vec![addr(None, "me@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Lunch".to_owned(),
        in_reply_to: Some("mid@example.test".to_owned()),
        references,
        rfc_message_id: rfc_message_id.map(str::to_owned),
        read: ReadState::Unread,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body: Body::Present {
            text: Some("original".to_owned()),
            raw: BlobId::generate(),
        },
        attachments: Vec::new(),
    }
}

/// The addresses of `list`, in order.
pub fn emails(list: &[Address]) -> Vec<&str> {
    list.iter().map(|addr| addr.email.as_str()).collect()
}
