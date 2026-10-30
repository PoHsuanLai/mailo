//! What the tests of `mail-crypto` share, declared once: the draft and sender a message to seal
//! is built from.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::new_account_id;
use mail_domain::{Address, Draft, DraftId, Identity, IdentityId, ReceiptRequest, SendState};

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
        openpgp: None,
        smime: None,
        priority: mail_domain::Priority::Normal,
        delivery: mail_domain::DeliveryReport::Unrequested,
        automatic: false,
        state: SendState::Editing,
        updated: at(),
    }
}
