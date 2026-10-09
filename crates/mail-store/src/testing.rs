//! What a test seeds a store with, and the few things only a test does to one.
//!
//! Behind the `test-support` feature, which a crate enables for its tests and the program never
//! does. Nothing above the store speaks SQL (`scripts/check-boundary.sh`), so a test that needs an
//! account, an identity or a label that no path of the program makes (a bare account whose plan
//! is `{}`, the oldest shape the fixtures wrote) asks for it here.

use crate::SqliteStore;
use chrono::{DateTime, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{
    AccountPlan, Identity, IdentityId, IsDefault, LabelId, LabelOrigin, MessageId, SyncCursor,
};
use parking_lot::ReentrantMutexGuard;
use porter_core::AccountId;
use rusqlite::{Connection, params};

/// A store in memory, with a scratch directory for its blobs that goes when it does.
pub fn in_memory() -> SqliteStore {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut store = SqliteStore::in_memory(dir.path()).expect("an in-memory store");
    store.scratch = Some(dir);
    store
}

/// An account with `address` and nothing else: its plan is `{}`, as the oldest fixtures wrote.
/// Left as it is if `id` is already there.
pub fn seed_account(store: &SqliteStore, id: AccountId, address: &str) {
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, '{}', datetime('now'))",
            params![id.to_string(), address],
        )
        .expect("seed an account");
}

/// An account with a real `plan`, created at `created` (now, when `None`), which is what orders
/// accounts. An account already there under `id` has its address and plan replaced.
pub fn seed_account_plan(
    store: &SqliteStore,
    id: AccountId,
    address: &str,
    plan: &AccountPlan,
    created: Option<DateTime<Utc>>,
) {
    let plan = serde_json::to_string(plan).expect("a plan encodes");
    let db = store.connection();
    let written = match created {
        Some(at) => db.execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET address = excluded.address, plan = excluded.plan",
            params![id.to_string(), address, plan, at.to_rfc3339()],
        ),
        None => db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET address = excluded.address, plan = excluded.plan",
            params![id.to_string(), address, plan],
        ),
    };
    written.expect("seed an account");
}

/// Take an account out of the store the way a delete of its row does, whatever the program
/// would have refused.
pub fn delete_account_row(store: &SqliteStore, id: AccountId) {
    store
        .connection()
        .execute("DELETE FROM accounts WHERE id = ?1", [id.to_string()])
        .expect("delete an account row");
}

/// An identity as it is, replacing the one with its id.
pub fn seed_identity(store: &SqliteStore, identity: &Identity) {
    store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                 is_default)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET account = excluded.account,
                 from_name = excluded.from_name, from_email = excluded.from_email,
                 reply_to = excluded.reply_to, signature = excluded.signature,
                 is_default = excluded.is_default",
            params![
                identity.id.to_string(),
                identity.account.to_string(),
                identity.from.name,
                identity.from.email,
                identity
                    .reply_to
                    .as_ref()
                    .map(|reply_to| serde_json::to_string(&Some(reply_to)).expect("encodes")),
                identity.signature,
                serde_json::to_string(&identity.default).expect("a default flag encodes"),
            ],
        )
        .expect("seed an identity");
}

/// The default identity of `account`, sending as `email`, named `name` if given.
pub fn seed_default_identity(
    store: &SqliteStore,
    account: AccountId,
    email: &str,
    name: Option<&str>,
) -> IdentityId {
    let id = IdentityId::generate();
    seed_identity_for(store, id, account, email, name);
    id
}

/// The default identity `id` of `account`, for a test that names it.
pub fn seed_identity_for(
    store: &SqliteStore,
    id: IdentityId,
    account: AccountId,
    email: &str,
    name: Option<&str>,
) {
    seed_identity(
        store,
        &Identity {
            id,
            account,
            from: mail_domain::Address {
                name: name.map(str::to_owned),
                email: email.to_owned(),
            },
            reply_to: None,
            signature: None,
            default: IsDefault::Default,
        },
    );
}

/// A label `id` with this `origin`, as the server or the user would have made it.
pub fn seed_label(
    store: &SqliteStore,
    id: LabelId,
    account: AccountId,
    name: &str,
    origin: LabelOrigin,
) {
    store
        .connection()
        .execute(
            "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, ?3, ?4)",
            params![
                id.to_string(),
                account.to_string(),
                name,
                serde_json::to_string(&origin).expect("an origin encodes"),
            ],
        )
        .expect("seed a label");
}

/// That a pass of `account` left its cursor for `mailbox` at `cursor`, finishing at `synced_at`
/// (`YYYY-MM-DD HH:MM:SS`, as SQLite writes it; now, when `None`).
pub fn seed_sync_state(
    store: &SqliteStore,
    account: AccountId,
    mailbox: &str,
    cursor: &SyncCursor,
    synced_at: Option<&str>,
) {
    store
        .connection()
        .execute(
            "INSERT INTO sync_state (account, mailbox, cursor, synced_at)
             VALUES (?1, ?2, ?3, COALESCE(?4, datetime('now')))
             ON CONFLICT(account, mailbox) DO UPDATE SET
                 cursor = excluded.cursor, synced_at = excluded.synced_at",
            params![
                account.to_string(),
                mailbox,
                serde_json::to_string(cursor).expect("a cursor encodes"),
                synced_at,
            ],
        )
        .expect("seed a sync state");
}

/// That `message` is `uid` of `mailbox` on the server (IMAP, `UIDVALIDITY` 1).
pub fn seed_remote_uid(
    store: &SqliteStore,
    account: AccountId,
    mailbox: &str,
    uid: u32,
    message: MessageId,
) {
    store
        .connection()
        .execute(
            "INSERT INTO remote_map (account, mailbox, uidvalidity, uid, message)
             VALUES (?1, ?2, 1, ?3, ?4)",
            params![account.to_string(), mailbox, uid, message.to_string()],
        )
        .expect("seed a remote address");
}

/// How many rows `table` holds.
pub fn count(store: &SqliteStore, table: &str) -> i64 {
    store
        .connection()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap_or_else(|why| panic!("count the rows of {table}: {why}"))
}

/// The id of every stored message, oldest first.
pub fn message_ids(store: &SqliteStore) -> Vec<MessageId> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT id FROM messages ORDER BY date, id")
        .expect("list the messages");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("list the messages")
        .map(|id| {
            id.expect("a message id")
                .parse::<uuid::Uuid>()
                .expect("an id")
        })
        .map(MessageId::from_uuid)
        .collect()
}

/// Every account id in the store, oldest first, as a test names them.
pub fn account_ids(store: &SqliteStore) -> Vec<AccountId> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT id FROM accounts ORDER BY created_at")
        .expect("list the accounts");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("list the accounts")
        .map(|id| id.expect("an id").parse::<uuid::Uuid>().expect("an id"))
        .map(account_id_from_uuid)
        .collect()
}

/// How many rows this connection has changed since it opened. A write that changes nothing
/// leaves it where it was, which is how a test tells that a repaint did not write.
pub fn total_changes(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap_or(0)
}

/// The store's writer, held: nothing else writes until this is dropped. Taken on the thread
/// that will hold it.
pub struct WriterHeld<'a>(#[allow(dead_code)] ReentrantMutexGuard<'a, Connection>);

impl std::fmt::Debug for WriterHeld<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriterHeld")
    }
}

/// Hold the store's writer, as a long write in the same process does.
pub fn hold_writer(store: &SqliteStore) -> WriterHeld<'_> {
    WriterHeld(store.connection())
}

/// Write a row the way another process does: through its own connection to the database at
/// `path`, which the store did not make.
pub fn seed_account_from_another_connection(path: &std::path::Path, id: &str) {
    let other = Connection::open(path).expect("open the database again");
    other
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?1, '{}', datetime('now'))",
            [id],
        )
        .expect("write from another connection");
}

impl SqliteStore {
    /// The connection itself, for a test of the store that has to look at its tables. Not for
    /// any crate above the store: a test there asks for what it wants in this module.
    pub fn raw_connection(&self) -> ReentrantMutexGuard<'_, Connection> {
        self.connection()
    }
}

impl SqliteStore {
    /// What accounts are selected `FROM` while held ones are set aside, for a test of that.
    pub fn raw_accounts(&self) -> &'static str {
        self.accounts()
    }
}

/// Rewrite stored messages: `edit` is shown each one and says whether it changed it, and a
/// changed message is written back the way any other change to it is.
pub fn edit_messages(store: &SqliteStore, mut edit: impl FnMut(&mut mail_domain::Message) -> bool) {
    use crate::Store;
    use mail_domain::{Change, ChangeId, Patch};
    for id in message_ids(store) {
        let mut message = store.message(id).expect("a stored message reads");
        if !edit(&mut message) {
            continue;
        }
        let account = message.account.clone();
        store
            .apply(
                account,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .expect("write the edited message back");
    }
}

/// Make every write to the outbox fail, as a full disk or a locked database would.
pub fn refuse_outbox_writes(store: &SqliteStore) {
    store
        .connection()
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'full'); END;",
        )
        .expect("refuse outbox writes");
}

/// Remove every identity of `account`, which is how an account added before identities were
/// created at setup looks.
pub fn delete_identities(store: &SqliteStore, account: AccountId) {
    store
        .connection()
        .execute(
            "DELETE FROM identities WHERE account = ?1",
            [account.to_string()],
        )
        .expect("delete identities");
}

/// The conversation of a stored message whose `Message-ID` begins with `prefix`, for a test that
/// knows what a send made it from but not the rest.
pub fn thread_with_rfc_prefix(store: &SqliteStore, prefix: &str) -> Option<mail_domain::ThreadId> {
    let found: Option<String> = store
        .connection()
        .query_row(
            "SELECT thread FROM messages WHERE rfc_message_id LIKE ?1 LIMIT 1",
            [format!("{prefix}%")],
            |r| r.get(0),
        )
        .ok();
    found
        .and_then(|id| id.parse::<uuid::Uuid>().ok())
        .map(mail_domain::ThreadId::from_uuid)
}

/// Every queued operation, due or not, in the order it was queued.
pub fn outbox_ops(store: &SqliteStore) -> Vec<mail_domain::ProtoOp> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT op FROM outbox ORDER BY id")
        .expect("list the outbox");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("list the outbox")
        .map(|op| serde_json::from_str(&op.expect("an op")).expect("a queued op decodes"))
        .collect()
}

/// Drop everything queued, as if it had all been sent.
pub fn clear_outbox(store: &SqliteStore) {
    store
        .connection()
        .execute("DELETE FROM outbox", [])
        .expect("clear the outbox");
}

/// How many rows name `account`, in `accounts` and in every table with an `account` column: what
/// is left of it in the store.
pub fn rows_naming(store: &SqliteStore, account: AccountId) -> i64 {
    let db = store.connection();
    let tables = table_names(&db);
    let id = account.to_string();
    let mut count: i64 = db
        .query_row("SELECT count(*) FROM accounts WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .expect("count the account");
    for table in tables {
        let has_account: i64 = db
            .query_row(
                &format!(
                    "SELECT count(*) FROM pragma_table_info('{table}') WHERE name = 'account'"
                ),
                [],
                |r| r.get(0),
            )
            .expect("read a table's columns");
        if has_account > 0 {
            count += db
                .query_row(
                    &format!("SELECT count(*) FROM \"{table}\" WHERE account = ?1"),
                    [&id],
                    |r| r.get::<_, i64>(0),
                )
                .expect("count a table's rows");
        }
    }
    count
}

/// Every text column of every table, joined with newlines: where a secret written to the
/// database would be.
pub fn all_text(store: &SqliteStore) -> String {
    let db = store.connection();
    let mut all = String::new();
    for name in table_names(&db) {
        let Ok(mut rows) = db.prepare(&format!("SELECT * FROM \"{name}\"")) else {
            continue;
        };
        let columns = rows.column_count();
        let mut query = rows.query([]).expect("read a table");
        while let Some(row) = query.next().expect("read a row") {
            for at in 0..columns {
                if let Ok(Some(text)) = row.get::<_, Option<String>>(at) {
                    all.push_str(&text);
                    all.push('\n');
                }
            }
        }
    }
    all
}

fn table_names(db: &Connection) -> Vec<String> {
    let mut stmt = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .expect("list the tables");
    stmt.query_map([], |r| r.get(0))
        .expect("list the tables")
        .map(|name| name.expect("a table name"))
        .collect()
}

/// Un-apply migration 0012, as a database last opened by the build before it would be, and every
/// migration after it, since the version is the highest one applied and a later one would leave
/// 0012 looking done. For a test that the next open does what the migration asks.
pub fn downgrade_to_before_reparse(db_path: &std::path::Path) {
    let db = Connection::open(db_path).expect("open the database");
    db.execute_batch(
        "DROP TABLE messages_to_reparse;
             DROP TABLE contacts; DROP TABLE contacts_counted; DROP TABLE contacts_sent;
             DROP TABLE address_books; DROP TABLE contacts_to_backfill;
             DROP TABLE templates;
             DROP TABLE invite_answers;
             DROP TABLE rules; DROP TABLE vacations;
             DROP TABLE pgp_keys; DROP TABLE autocrypt_peers;
             ALTER TABLE drafts DROP COLUMN openpgp;
             DROP TABLE smime_certs;
             ALTER TABLE drafts DROP COLUMN smime;
             ALTER TABLE outbox DROP COLUMN messages;
             DROP TRIGGER unplaced_found_ins;
             DROP TRIGGER unplaced_found_upd;
             DROP TABLE unplaced;
             ALTER TABLE threads DROP COLUMN mute;
             ALTER TABLE thread_summary DROP COLUMN mute;
             DROP TABLE contact_groups;
             DROP TABLE views;
             DROP TABLE destroyed;
             DROP INDEX thread_summary_follow_up;
             ALTER TABLE threads DROP COLUMN follow_up;
             ALTER TABLE thread_summary DROP COLUMN follow_up;
             DROP TABLE follow_up_held;
             DROP TABLE found_on_server;
             DROP TABLE secrets_adopted;
             CREATE TABLE views (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
                 filter TEXT NOT NULL, sort TEXT NOT NULL, group_by TEXT,
                 threading TEXT NOT NULL, shown TEXT NOT NULL, hover TEXT NOT NULL,
                 position INTEGER NOT NULL);
             CREATE INDEX views_position ON views(position);
             DELETE FROM schema_version WHERE version >= 12;",
    )
    .expect("un-apply the migrations");
}

/// What the server was observed to support, written as a sync does.
pub fn seed_caps(
    store: &SqliteStore,
    account: AccountId,
    caps: &mail_domain::AccountCaps,
    at: DateTime<Utc>,
) -> Result<(), crate::StoreError> {
    use crate::Store;
    store.put_caps(account, caps, at)
}
