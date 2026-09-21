//! The real [`Store`]: SQLite in WAL mode, with FTS5.

mod outbox;
mod read;
mod row;
mod write;

use crate::blob::BlobStore;
use crate::{StoreError, migrate};
use rusqlite::Connection;
use std::path::Path;

/// A connection to the on-disk database, plus the blob store beside it.
///
/// One connection, not a pool. Every method takes `&self` and SQLite serializes writes anyway;
/// a pool would buy parallel reads at the cost of having to reason about a writer and a reader
/// disagreeing about what a thread's summary says. Revisit when a profile says to.
#[derive(Debug)]
pub struct SqliteStore {
    db: Connection,
    blobs: BlobStore,
}

impl SqliteStore {
    /// Open or create the database at `db_path`, with blobs under `blob_root`.
    pub fn open(
        db_path: impl AsRef<Path>,
        blob_root: impl AsRef<Path>,
    ) -> Result<Self, StoreError> {
        let db = Connection::open(db_path).map_err(|e| StoreError::Db(e.to_string()))?;
        Self::from_connection(db, blob_root)
    }

    /// An in-memory database. Tests only: it vanishes when dropped.
    pub fn in_memory(blob_root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db = Connection::open_in_memory().map_err(|e| StoreError::Db(e.to_string()))?;
        Self::from_connection(db, blob_root)
    }

    fn from_connection(db: Connection, blob_root: impl AsRef<Path>) -> Result<Self, StoreError> {
        // WAL lets a reader run while a writer commits, which is what keeps the UI responsive
        // during a sync. NORMAL trades a fsync per commit for the small risk of losing the
        // last transaction on power loss — acceptable, because the server still has the mail.
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .map_err(|e| StoreError::Db(e.to_string()))?;
        migrate::migrate(&db)?;
        Ok(Self {
            db,
            blobs: BlobStore::new(blob_root.as_ref().to_path_buf()),
        })
    }

    /// The blob store, for callers that need to read a raw message or an attachment.
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// The underlying connection, for the blob store and for tests.
    pub fn connection(&self) -> &Connection {
        &self.db
    }
}

use crate::{OutboxEntry, Settle, Store, sql};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, Cursor, Filter, Ingest, Message, MessageId, OutboxId, Page, Patch, Property, Query,
    RemoteIntent, SortDir, Thread, ThreadId, ThreadSummary,
};

/// The `thread_summary` column a [`Property`] sorts on.
///
/// Every one is indexed or cheap; `Size` is not stored, so it falls back to date rather than
/// forcing a scan over message bodies to answer a sort nobody can see the result of anyway.
fn sort_column(property: Property) -> &'static str {
    match property {
        Property::Date | Property::Size => "ts.last_date",
        Property::Subject => "ts.subject",
        Property::From | Property::Sender => "ts.from_email",
        Property::Attachments => "ts.attachments",
        Property::Pin => "ts.pin",
    }
}

/// Encode a keyset position. Opaque to every caller; the format is this module's business.
fn encode_cursor(sort_value: &str, thread: ThreadId) -> Cursor {
    Cursor(format!("{}\u{1f}{}", sort_value, thread))
}

fn decode_cursor(cursor: &Cursor) -> Result<(String, String), StoreError> {
    cursor
        .0
        .split_once('\u{1f}')
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .ok_or(StoreError::BadCursor)
}

impl Store for SqliteStore {
    fn threads(
        &self,
        query: &Query,
        now: DateTime<Utc>,
    ) -> Result<Page<ThreadSummary>, StoreError> {
        let compiled = sql::compile(&query.filter, now);
        let column = sort_column(query.sort.property);
        let dir = match query.sort.dir {
            SortDir::Asc => "ASC",
            SortDir::Desc => "DESC",
        };
        // Keyset, not OFFSET: an offset shifts under the reader when mail arrives mid-scroll,
        // which silently skips or repeats a row. The tie-break on thread id makes the order
        // total, so a page boundary is never ambiguous.
        let (keyset, mut params): (String, Vec<sql::SqlValue>) = match &query.page.after {
            None => (String::from("1=1"), Vec::new()),
            Some(cursor) => {
                let (value, thread) = decode_cursor(cursor)?;
                let cmp = if matches!(query.sort.dir, SortDir::Asc) {
                    ">"
                } else {
                    "<"
                };
                (
                    format!("({column}, ts.thread) {cmp} (?, ?)"),
                    vec![sql::SqlValue::Text(value), sql::SqlValue::Text(thread)],
                )
            }
        };

        let sql_text = format!(
            "SELECT {} FROM thread_summary ts WHERE {} AND {} ORDER BY {column} {dir}, ts.thread {dir} LIMIT ?",
            read::SUMMARY_COLUMNS.replace("thread,", "ts.thread,"),
            compiled.where_clause,
            keyset,
        );

        let mut bound: Vec<sql::SqlValue> = compiled.params;
        bound.append(&mut params);
        // One extra row tells us whether another page exists without a second COUNT query.
        let limit = i64::from(query.page.limit);
        bound.push(sql::SqlValue::Int(limit + 1));

        let mut stmt = self.db.prepare(&sql_text)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(bound.iter().map(|v| match v {
            sql::SqlValue::Text(t) => rusqlite::types::Value::Text(t.clone()),
            sql::SqlValue::Int(i) => rusqlite::types::Value::Integer(*i),
        })))?;

        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(self.read_summary(row)?);
        }

        let next = if items.len() as i64 > limit {
            items.truncate(limit as usize);
            items.last().map(|s| {
                let value = match query.sort.property {
                    Property::Subject => s.subject.clone(),
                    Property::From | Property::Sender => s.from.email.clone(),
                    _ => row::from_time(s.last_date),
                };
                encode_cursor(&value, s.id)
            })
        } else {
            None
        };
        Ok(Page { items, next })
    }

    fn count(&self, filter: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError> {
        let compiled = sql::compile(filter, now);
        let sql_text = format!(
            "SELECT count(*) FROM thread_summary ts WHERE {}",
            compiled.where_clause
        );
        let n: i64 = self.db.query_row(
            &sql_text,
            rusqlite::params_from_iter(compiled.params.iter().map(|v| match v {
                sql::SqlValue::Text(t) => rusqlite::types::Value::Text(t.clone()),
                sql::SqlValue::Int(i) => rusqlite::types::Value::Integer(*i),
            })),
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    fn thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        self.load_thread(id)
    }

    fn message(&self, id: MessageId) -> Result<Message, StoreError> {
        let sql_text = format!(
            "SELECT {} FROM messages WHERE id = ?1",
            read::MESSAGE_COLUMNS
        );
        let mut stmt = self.db.prepare_cached(&sql_text)?;
        let mut rows = stmt.query(rusqlite::params![id.to_string()])?;
        match rows.next()? {
            Some(row) => self.read_message(row),
            None => Err(StoreError::NoMessage(id)),
        }
    }

    fn apply(&self, _account: AccountId, patch: &Patch) -> Result<(), StoreError> {
        self.write_patch(patch)
    }

    fn ingest(&self, account: AccountId, ingest: Ingest) -> Result<Patch, StoreError> {
        self.write_ingest(account, ingest)
    }

    fn enqueue(
        &self,
        account: AccountId,
        intent: RemoteIntent,
        undo: &Patch,
        now: DateTime<Utc>,
    ) -> Result<Option<OutboxId>, StoreError> {
        self.queue(account, intent, undo, now)
    }

    fn outbox_due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError> {
        self.due(account, now)
    }

    fn outbox_settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.settle(id, settle, now)
    }
}
