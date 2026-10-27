//! What the tests in this binary build their stores and mail from.
//!
//! The rows only a test writes come from `mail_store::testing`; this is what the files here share
//! on top of it: the two accounts, the clock, one plain message and how it arrives, and the run
//! of one scenario on both stores that the parity tests compare.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::testing::{in_memory, seed_account};
use mail_store::{MemoryStore, SqliteStore, Store};
use porter_core::AccountId;

pub fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

pub fn acct_other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}

/// `n` seconds into the tests' day.
pub fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

/// A SQLite store in memory, with the rows of both accounts its foreign keys need.
pub fn sqlite() -> SqliteStore {
    let store = in_memory();
    seed_account(&store, acct_account(), "me@example.test");
    seed_account(&store, acct_other(), "also@example.test");
    store
}

/// Both stores holding `messages`, the SQLite one in a file in `dir` so a test can open it again.
pub fn both_on_disk(
    dir: &std::path::Path,
    messages: impl IntoIterator<Item = Message>,
) -> (SqliteStore, MemoryStore) {
    let sqlite = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    seed_account(&sqlite, acct_account(), "me@example.test");
    let memory = MemoryStore::new();
    for message in messages {
        let upsert = patch(vec![Change::MessageUpsert(Box::new(message))]);
        sqlite.apply(acct_account(), &upsert).unwrap();
        memory.apply(acct_account(), &upsert).unwrap();
    }
    (sqlite, memory)
}

/// Both stores side by side, for a test that asks each the same thing.
pub struct Both {
    pub sqlite: SqliteStore,
    pub memory: MemoryStore,
}

impl Both {
    /// The SQLite store of [`sqlite`] beside an empty memory store.
    pub fn new() -> Both {
        Both {
            sqlite: sqlite(),
            memory: MemoryStore::new(),
        }
    }

    /// Each store, named for a failure message.
    pub fn each(&self) -> [(&'static str, &dyn Store); 2] {
        [("sqlite", &self.sqlite), ("memory", &self.memory)]
    }

    /// Run `f` on both stores and insist they answer alike.
    pub fn agree<T: PartialEq + std::fmt::Debug>(&self, f: impl Fn(&dyn Store) -> T) -> T {
        let (sqlite, memory) = (f(&self.sqlite), f(&self.memory));
        assert_eq!(sqlite, memory, "SQLite and memory disagree");
        sqlite
    }
}

/// Run `scenario` on both stores and hand back what each produced.
pub fn both<T>(scenario: impl Fn(&dyn Store) -> T) -> (T, T) {
    let sqlite = sqlite();
    let memory = MemoryStore::new();
    (scenario(&sqlite), scenario(&memory))
}

pub fn imap(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 7,
        uid,
    }
}

pub fn graph(mailbox: &str, id: &str) -> RemoteRef {
    RemoteRef::Graph {
        mailbox: mailbox.to_owned(),
        id: id.to_owned(),
    }
}

pub fn mailbox(path: &str) -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: path.to_owned(),
    }
}

/// Message `n` as a header fetch into the inbox builds it: in thread `n + 1000`, sent `n` seconds
/// in, no body yet.
pub fn message(n: u128) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000)),
        account: acct_account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

/// A sync of the mailbox at `path` that found nothing yet.
pub fn ingest(path: &str) -> Ingest {
    Ingest {
        mailbox: mailbox(path),
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![],
        flags: vec![],
        flag_colors: Vec::new(),
        labels: vec![],
        label_names: Vec::new(),
        provider_threads: Vec::new(),
        gone: vec![],
    }
}

/// `m` arrives at `remote`, in the mailbox the address names.
pub fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    let path = match &remote {
        RemoteRef::Imap { mailbox, .. } | RemoteRef::Graph { mailbox, .. } => mailbox.clone(),
        RemoteRef::Pop { .. } | RemoteRef::Jmap { .. } => "INBOX".to_owned(),
    };
    let mut batch = ingest(&path);
    batch.messages.push(Fetched {
        remote,
        key: m.key.clone(),
        raw: BlobId::generate(),
        message: m.clone(),
    });
    store.ingest(acct_account(), batch).unwrap();
}

pub fn patch(changes: Vec<Change>) -> Patch {
    Patch {
        id: ChangeId::generate(),
        changes,
    }
}

/// The first `limit` threads `filter` lists, newest first.
pub fn query(filter: Filter, limit: u32) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq { after: None, limit },
    }
}
