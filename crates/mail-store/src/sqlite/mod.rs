//! The real [`Store`]: SQLite in WAL mode, with FTS5.

mod draft;
mod outbox;
mod read;
mod row;
mod write;

use crate::blob::BlobStore;
use crate::{StoreError, migrate};
use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// A connection to the on-disk database, plus the blob store beside it.
///
/// One connection, not a pool. Every method takes `&self` and SQLite serializes writes anyway;
/// a pool would buy parallel reads at the cost of having to reason about a writer and a reader
/// disagreeing about what a thread's summary says. Revisit when a profile says to.
#[derive(Debug)]
pub struct SqliteStore {
    /// Behind a mutex because `rusqlite::Connection` is `Send` but **not `Sync`**, so an
    /// `Arc<SqliteStore>` shared between the UI thread and a sync task would not compile
    /// without it. SQLite serialises writes regardless, and WAL's concurrent readers are
    /// concurrent across *connections*, so one guarded connection is honest about what is
    /// actually happening rather than pretending at parallelism a single handle cannot give.
    ///
    /// **Reentrant**, and that is not an optimisation. `write_patch` takes the connection to
    /// open a transaction and then calls `write_change`, which needs it again; with a plain
    /// `std::sync::Mutex` that is a deadlock — the process simply stops, with no error and no
    /// panic. A reentrant lock is sound here because `rusqlite` only ever needs `&Connection`,
    /// so recursion hands out a second shared reference rather than aliasing a mutable one.
    db: ReentrantMutex<Connection>,
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
            db: ReentrantMutex::new(db),
            blobs: BlobStore::new(blob_root.as_ref().to_path_buf()),
        })
    }

    /// The blob store, for callers that need to read a raw message or an attachment.
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// The underlying connection.
    ///
    /// Reentrant: taking it twice on one thread is normal here, because a public method opens a
    /// transaction and then calls helpers that each need the connection again. With a plain
    /// mutex that is a deadlock, and a deadlock has no error message.
    pub fn connection(&self) -> ReentrantMutexGuard<'_, Connection> {
        self.db.lock()
    }
}

use crate::{OutboxEntry, Settle, Store, sql};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, Cursor, Draft, DraftId, Filter, Ingest, MailboxRef, Message, MessageId, OutboxId,
    Page, Patch, Property, Query, RemoteIntent, RemoteRef, SendState, SortDir, SyncCursor, Thread,
    ThreadId, ThreadSummary,
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

        let db = self.connection();

        let mut stmt = db.prepare(&sql_text)?;
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
        let n: i64 = self.connection().query_row(
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
        let db = self.connection();
        let mut stmt = db.prepare_cached(&sql_text)?;
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

    fn unfetched(
        &self,
        account: AccountId,
        limit: u32,
    ) -> Result<Vec<mail_domain::RemoteRef>, StoreError> {
        // A message may have several remote addresses; any one of them can fetch the body, so
        // take the first per message rather than returning the same work several times.
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT r.mailbox, r.uidvalidity, r.uid, r.uidl
             FROM messages m
             JOIN remote_map r ON r.message = m.id
             WHERE m.account = ?1 AND m.body_raw IS NULL
             GROUP BY m.id
             ORDER BY m.date
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![account.to_string(), i64::from(limit)],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (mailbox, uidvalidity, uid, uidl) = row?;
            out.push(match (uid, uidl) {
                (Some(uid), None) => RemoteRef::Imap {
                    mailbox,
                    uidvalidity: uidvalidity.unwrap_or(0) as u32,
                    uid: uid as u32,
                },
                (None, Some(uidl)) => RemoteRef::Pop { uidl },
                _ => {
                    return Err(StoreError::Decode {
                        what: "remote_map row".to_owned(),
                        why: "row has neither a uid nor a uidl".to_owned(),
                    });
                }
            });
        }
        Ok(out)
    }

    fn cursor(&self, mailbox: &MailboxRef) -> Result<Option<SyncCursor>, StoreError> {
        // `sync_state` has been written by every ingest since the store was created. Nothing
        // had ever read it back, which is why the CONDSTORE path could never start.
        let text: Option<String> = self
            .connection()
            .query_row(
                "SELECT cursor FROM sync_state WHERE account = ?1 AND mailbox = ?2",
                rusqlite::params![mailbox.account.to_string(), mailbox.path],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|t| row::json("SyncCursor", &t)).transpose()
    }

    fn remote_refs(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare_cached(
            "SELECT uidvalidity, uid, uidl FROM remote_map
             WHERE account = ?1 AND mailbox = ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![mailbox.account.to_string(), mailbox.path],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (uidvalidity, uid, uidl) = row?;
            out.push(match (uid, uidl) {
                (Some(uid), None) => RemoteRef::Imap {
                    mailbox: mailbox.path.clone(),
                    uidvalidity: uidvalidity.unwrap_or(0) as u32,
                    uid: uid as u32,
                },
                (None, Some(uidl)) => RemoteRef::Pop { uidl },
                _ => {
                    return Err(StoreError::Decode {
                        what: "remote_map row".to_owned(),
                        why: "row has neither a uid nor a uidl".to_owned(),
                    });
                }
            });
        }
        Ok(out)
    }

    fn draft(&self, id: DraftId) -> Result<Draft, StoreError> {
        self.load_draft(id)
    }

    fn drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError> {
        self.load_drafts(account)
    }

    fn set_send_state(
        &self,
        id: DraftId,
        state: &SendState,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        SqliteStore::set_send_state(self, id, state, now)
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
