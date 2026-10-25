//! The store every native test starts from: one account with its identity and capabilities, and
//! an inbox of mail the test names.
//!
//! Each test file used to spell this out for itself, and the copies differed only in the
//! Message-ID prefix and the inbox they held. A test that asserts a message id keeps its prefix
//! through [`Seed::prefix`]; the rest is written here once.

#![allow(dead_code)]

use mail_core::{Arrival, SqliteStore, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::path::Path;
use std::sync::Arc;

/// The account every native test's store holds.
pub fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

/// What was observed against a label server: archiving drops the inbox label.
pub fn gmail_caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::DropInbox,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Yes,
        connections: ConnectionBudget::default(),
        observed_at: chrono::Utc::now(),
    }
}

/// What a seeded message says below its headers.
#[derive(Clone, Copy)]
pub enum Body {
    /// "The body of {subject}.", so a test can tell the letters apart by their text.
    OfSubject,
    /// "The body.", the same in every letter.
    Plain,
}

/// What [`seeded_with`] writes. The defaults are the common store: `me@example.test` with
/// capabilities, and an empty inbox.
pub struct Seed<'a> {
    /// The account's id.
    pub account: AccountId,
    /// The account's own address.
    pub address: &'a str,
    /// The capabilities `account add` writes. `None` is a store without them.
    pub caps: Option<AccountCaps>,
    /// Newest first: (sender, subject). Each is an hour older than the one before it.
    pub inbox: &'a [(&'a str, &'a str)],
    /// How each message's Message-ID and UIDL begin: `{prefix}{n}`.
    pub prefix: &'a str,
    pub body: Body,
}

impl Default for Seed<'_> {
    fn default() -> Self {
        Seed {
            account: acct_account(),
            address: "me@example.test",
            caps: Some(gmail_caps()),
            inbox: &[],
            prefix: "seed",
            body: Body::OfSubject,
        }
    }
}

/// An empty store under `dir` with `account`, its identity and, if given, its capabilities.
///
/// The capabilities are written because `account add` always writes them, and a store without
/// them is a state the application cannot reach (only the tests that do not archive go without).
pub fn account(
    dir: &Path,
    account: AccountId,
    address: &str,
    caps: Option<&AccountCaps>,
) -> SqliteStore {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    add_account(&store, account.clone(), address);
    mail_store::testing::seed_identity_for(
        &store,
        IdentityId::generate(),
        account.clone(),
        address,
        None,
    );
    if let Some(caps) = caps {
        mail_store::testing::seed_caps(&store, account, caps, chrono::Utc::now()).unwrap();
    }
    store
}

/// `account` at `address` in `store`, with nothing else of it: no identity, no capabilities.
pub fn add_account(store: &SqliteStore, account: AccountId, address: &str) {
    mail_store::testing::seed_account(store, account, address);
}

/// Deliver `raw` into `account`'s Inbox as POP3 does, `uidl` naming it.
pub fn deliver(store: &SqliteStore, account: AccountId, uidl: &str, raw: String) {
    absorb(
        store,
        account.clone(),
        MailboxRef {
            account,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        chrono::Utc::now(),
    )
    .unwrap();
}

/// A store in `dir` holding what `seed` says.
pub fn seeded_with(dir: &Path, seed: Seed) -> Arc<SqliteStore> {
    let store = account(dir, seed.account.clone(), seed.address, seed.caps.as_ref());
    let now = chrono::Utc::now();
    for (n, (from, subject)) in seed.inbox.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let prefix = seed.prefix;
        let text = match seed.body {
            Body::OfSubject => format!("The body of {subject}."),
            Body::Plain => "The body.".to_owned(),
        };
        let raw = format!(
            "From: {from}\r\nTo: {}\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <{prefix}{n}@example.test>\r\n\r\n{text}\r\n",
            seed.address
        );
        deliver(&store, seed.account.clone(), &format!("{prefix}{n}"), raw);
    }
    Arc::new(store)
}
