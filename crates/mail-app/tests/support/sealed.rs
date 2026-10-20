//! A draft sent through the library and read back as the outbox holds it, for the tests of what
//! OpenPGP and S/MIME make of it. The two files differ in the account, the clock, and the one
//! field of the draft that asks for sealing; the rest is written here once.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use mail_app::cli;
use mail_core::compose;
use mail_core::testing::MapSigningStore;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_core::AccountId;

/// `addresses` as bare recipients.
pub fn to(addresses: &[&str]) -> Vec<Address> {
    addresses
        .iter()
        .map(|email| Address {
            name: None,
            email: (*email).to_owned(),
        })
        .collect()
}

/// A saved draft of "Secret plans" to `recipients`, with `bcc` blind, after `ask` has set what it
/// asks of the sender's keys.
pub fn draft(
    store: &SqliteStore,
    account: AccountId,
    now: DateTime<Utc>,
    ask: impl FnOnce(&mut Draft),
    recipients: &[&str],
    bcc: &[&str],
) -> Draft {
    let mut draft = compose::draft_new(
        store,
        account,
        &to(recipients),
        "Secret plans",
        "meet at the usual place",
        now,
    )
    .unwrap();
    ask(&mut draft);
    draft.bcc = to(bcc);
    compose::save(store, &draft).unwrap();
    draft
}

/// What the outbox holds for `account`, due a year on.
pub fn submissions(
    store: &SqliteStore,
    account: AccountId,
    now: DateTime<Utc>,
) -> Vec<mail_store::OutboxEntry> {
    store
        .outbox_due(account, now + chrono::TimeDelta::try_days(365).unwrap())
        .unwrap()
}

/// The frozen bytes of the only queued submission.
pub fn frozen(store: &SqliteStore, account: AccountId, now: DateTime<Utc>) -> Vec<u8> {
    let entries = submissions(store, account, now);
    assert_eq!(entries.len(), 1, "one submission queued");
    let ProtoOp::Submit { raw, .. } = &entries[0].op else {
        panic!("{:?}", entries[0].op);
    };
    store.blobs().get(*raw).unwrap()
}

/// Send `draft` as the command line does, and say what it printed or why it refused.
pub fn send(
    store: &SqliteStore,
    secrets: &MapSigningStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<String, String> {
    compose::send_with(store, secrets, &mail_core::pgp::no_passphrase, draft, now)
        .map(|(draft, post)| cli::compose::queued(&draft, &post))
        .map_err(|e| e.to_string())
}
