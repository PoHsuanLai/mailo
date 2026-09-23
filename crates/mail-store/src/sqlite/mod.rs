//! The real [`Store`]: SQLite in WAL mode, with FTS5.

mod draft;
mod folders;
mod outbox;
mod read;
mod receipt;
mod reparse;
mod row;
mod search;
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
///
/// **The profile, since it now exists** (`tests/concurrency.rs`): a reader looping on the list
/// query alongside a sync writing nine 200-message batches completed 23 reads — roughly ten
/// repaints a second while mail is absorbing. Reads and writes take turns for the length of a
/// batch, which is visible as a list that updates in steps during a sync rather than smoothly.
///
/// Phase 8b changed that answer, and the worry above turned out to be the right one to have: a
/// second connection reading a half-written thread summary *is* a wrong answer, so the readers
/// are never used inside a write. What keeps that true is a signature rather than a rule —
/// `summary_of`, `messages_of`, `labels_of` and `read_message` take the connection they should
/// read from, so a write path physically cannot hand them a reader.
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
    /// Connections that only ever read — `plan.md` phase 8b.
    ///
    /// WAL gives concurrent readers, but concurrent *across connections*: with one handle every
    /// query in this process waits behind whatever is committing, and the comment above the
    /// pragmas claimed a benefit this struct could not deliver. These are what make it true.
    ///
    /// Empty for an in-memory database, which is not a file two connections can share — a second
    /// `:memory:` handle opens a second, empty database. Tests fall back to the writer, which is
    /// what they already had.
    ///
    /// A bounded `Vec` rather than one per thread, so a process that spawns tasks freely cannot
    /// open file handles without limit; `try_lock` in turn takes the first free one.
    readers: Vec<ReentrantMutex<Connection>>,
    blobs: BlobStore,
}

/// How many read-only connections to open beside the writer.
///
/// Three: the window's list, whatever the reader pane is showing, and a sync pass asking what it
/// already holds. More would be file handles against a mailbox that belongs to one person.
const READERS: usize = 3;

impl SqliteStore {
    /// Open or create the database at `db_path`, with blobs under `blob_root`.
    pub fn open(
        db_path: impl AsRef<Path>,
        blob_root: impl AsRef<Path>,
    ) -> Result<Self, StoreError> {
        let path = db_path.as_ref().to_path_buf();
        let db = Connection::open(&path).map_err(|e| StoreError::Db(e.to_string()))?;
        let mut store = Self::from_connection(db, blob_root)?;
        // After `from_connection`, which is what runs the migrations: a reader opened against an
        // unmigrated file would be a connection whose schema does not exist yet.
        for _ in 0..READERS {
            let reader = Connection::open(&path).map_err(|e| StoreError::Db(e.to_string()))?;
            reader
                .execute_batch(
                    "PRAGMA journal_mode = WAL;
                     PRAGMA busy_timeout = 5000;",
                )
                .map_err(|e| StoreError::Db(e.to_string()))?;
            // Before `query_only`: a TEMP vocab table is per connection, and a query-only
            // connection is not allowed to create one.
            search::ensure_vocab(&reader)?;
            reader
                .execute_batch("PRAGMA query_only = ON;")
                .map_err(|e| StoreError::Db(e.to_string()))?;
            store.readers.push(ReentrantMutex::new(reader));
        }
        Ok(store)
    }

    /// An in-memory database. Tests only: it vanishes when dropped.
    pub fn in_memory(blob_root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db = Connection::open_in_memory().map_err(|e| StoreError::Db(e.to_string()))?;
        Self::from_connection(db, blob_root)
    }

    fn from_connection(db: Connection, blob_root: impl AsRef<Path>) -> Result<Self, StoreError> {
        // WAL lets a reader run while a writer commits, which is what keeps the UI responsive
        // during a sync — and that is true across *connections*, so until phase 8b opened the
        // read-only ones beside this handle, the sentence described SQLite rather than this
        // program. NORMAL trades a fsync per commit for the small risk of losing the
        // last transaction on power loss — acceptable, because the server still has the mail.
        //
        // `busy_timeout` is what happens when two *writers* meet, which WAL does not help with:
        // SQLite serialises them, and the second either waits or is told "database is locked".
        // Two writers is not an edge case here — `mailo sync` in a terminal while the window is
        // open is an ordinary thing to do, and so is a scheduled sync overlapping a manual one.
        //
        // Five seconds against transactions that are one fetch batch long, which the scale
        // tests measure in milliseconds. Written down rather than inherited: `rusqlite` happens
        // to default to the same five seconds, and a default that nothing names is a behaviour
        // nobody notices changing.
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| StoreError::Db(e.to_string()))?;
        migrate::migrate(&db)?;
        backfill_fts(&db)?;
        search::ensure_vocab(&db)?;
        let store = Self {
            db: ReentrantMutex::new(db),
            readers: Vec::new(),
            blobs: BlobStore::new(blob_root.as_ref().to_path_buf()),
        };
        store.refresh_queued_summaries()?;
        Ok(store)
    }

    /// Re-derive every summary a migration queued, then forget them.
    ///
    /// For a derivation change SQL cannot express; see `0007_resummarize.sql`. Nothing queued is
    /// one indexed count and no work.
    fn refresh_queued_summaries(&self) -> Result<(), StoreError> {
        let queued: Vec<String> = {
            let db = self.connection();
            let mut stmt = db.prepare("SELECT thread FROM summaries_to_refresh")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        if queued.is_empty() {
            return Ok(());
        }
        // Held for the whole pass: the lock is reentrant, so `refresh_summary` takes it again
        // inside this transaction rather than waiting on it.
        let db = self.connection();
        let tx = db
            .unchecked_transaction()
            .map_err(|e| StoreError::Db(e.to_string()))?;
        for thread in queued {
            let id: uuid::Uuid = thread
                .parse()
                .map_err(|e: uuid::Error| StoreError::Decode {
                    what: "summaries_to_refresh.thread".to_owned(),
                    why: e.to_string(),
                })?;
            // One thread that no longer decodes keeps the summary it had. Failing here would
            // stop the database opening at all, over one row, on the first run after an upgrade.
            let _ = self.refresh_summary(mail_domain::ThreadId::from_uuid(id));
        }
        tx.execute("DELETE FROM summaries_to_refresh", [])?;
        tx.commit().map_err(|e| StoreError::Db(e.to_string()))?;
        Ok(())
    }
}

/// Mark the remote part `section` of `attachments` as held in `blob`. Shared by both stores so
/// "which attachment is that section" has one answer.
pub(crate) fn held(
    attachments: &mut [mail_domain::Attachment],
    message: MessageId,
    section: &str,
    blob: mail_domain::BlobId,
    size: u64,
) -> Result<(), StoreError> {
    let part = attachments
        .iter_mut()
        .find(|a| matches!(&a.content, mail_domain::PartContent::Remote { section: s } if s == section))
        .ok_or_else(|| StoreError::NoPart {
            message,
            section: section.to_owned(),
        })?;
    part.content = mail_domain::PartContent::Held(blob);
    part.size = size;
    Ok(())
}

/// Fill `messages.fts_text` for rows that predate migration 0004, or that a later one cleared.
///
/// The migration could not: segmenting a run of ideographs into bigrams is not something SQL can
/// express, and the whole point of that migration is that the indexed text is no longer the
/// message columns. So it runs here, once, over the rows that have no segmented text yet — and
/// the `AFTER UPDATE` trigger reindexes each one as it is written.
///
/// Cheap to check and cheap to skip: one indexed-free count on a column that is `NOT NULL` for
/// every row after the first run. A 2372-message maildrop takes a fraction of a second; a
/// database that has already been backfilled does no work at all.
fn backfill_fts(db: &Connection) -> Result<(), StoreError> {
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE fts_text IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| StoreError::Db(e.to_string()))?;
    if pending == 0 {
        return Ok(());
    }

    /// One row's worth of the columns the index is built from.
    type Indexed = (i64, String, Option<String>, String, String, Option<String>);
    let rows: Vec<Indexed> = {
        let mut stmt = db
            .prepare(
                "SELECT rowid, subject, from_name, from_email, recipients, body_text
                 FROM messages WHERE fts_text IS NULL",
            )
            .map_err(|e| StoreError::Db(e.to_string()))?;
        let mapped = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(|e| StoreError::Db(e.to_string()))?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StoreError::Db(e.to_string()))?
    };

    let tx = db
        .unchecked_transaction()
        .map_err(|e| StoreError::Db(e.to_string()))?;
    for (rowid, subject, from_name, from_email, recipients, body_text) in rows {
        // A row whose recipients no longer decode is indexed without them, rather than stopping
        // the database from opening over one message.
        let recipients: read::Recipients =
            row::json("Message.recipients", &recipients).unwrap_or_default();
        let from = mail_domain::Address {
            name: from_name,
            email: from_email,
        };
        let text = crate::sql::message_index(
            &subject,
            &from,
            &recipients.to,
            &recipients.cc,
            body_text.as_deref(),
        );
        tx.execute(
            "UPDATE messages SET fts_text = ?2 WHERE rowid = ?1",
            rusqlite::params![rowid, text],
        )
        .map_err(|e| StoreError::Db(e.to_string()))?;
    }
    tx.commit().map_err(|e| StoreError::Db(e.to_string()))?;
    Ok(())
}

impl SqliteStore {
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

    /// A connection for reading, which is not the one that writes.
    ///
    /// The first free reader, or the writer when there are none — an in-memory database has
    /// none, because a second `:memory:` handle is a second, empty database rather than a second
    /// view of this one.
    ///
    /// **Never inside a write.** A reader is a different connection, so it sees the last
    /// committed state and not the transaction in progress; a helper reading through this while
    /// `write_patch` had a transaction open would answer with the world as it was before the
    /// change it is part of. That is why `summary_of`, `messages_of`, `labels_of` and
    /// `read_message` take a `&Connection` instead of reaching for one: the caller says which
    /// world it means, and the compiler will not let a write path forget.
    pub fn reader(&self) -> ReentrantMutexGuard<'_, Connection> {
        for reader in &self.readers {
            if let Some(free) = reader.try_lock() {
                return free;
            }
        }
        // Every reader busy: wait on one of them rather than on the writer, which may hold its
        // lock for a whole ingest batch.
        match self.readers.first() {
            Some(reader) => reader.lock(),
            None => self.db.lock(),
        }
    }
}

use crate::{OutboxEntry, Settle, Store, Term, sql};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, Cursor, Draft, DraftId, Filter, Ingest, MailboxRef, Message, MessageId,
    OutboxId, Page, Patch, Property, Query, RemoteIntent, RemoteRef, SendState, SortDir,
    SyncCursor, Thread, ThreadId, ThreadSummary,
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

        let db = self.reader();

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

    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<Term>, StoreError> {
        let db = self.reader();
        search::terms_with_prefix(&db, prefix, limit)
    }

    fn top_hits(
        &self,
        filter: &Filter,
        k: usize,
        window: &[ThreadId],
        _now: DateTime<Utc>,
    ) -> Result<Vec<(ThreadSummary, f64)>, StoreError> {
        // `now` is unused: the window already answered every clause that reads the clock.
        let db = self.reader();
        search::top_hits(self, &db, filter, k, window)
    }

    fn count(&self, filter: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError> {
        let compiled = sql::compile(filter, now);
        let sql_text = format!(
            "SELECT count(*) FROM thread_summary ts WHERE {}",
            compiled.where_clause
        );
        let n: i64 = self.reader().query_row(
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
        let db = self.reader();
        let mut stmt = db.prepare_cached(&sql_text)?;
        let mut rows = stmt.query(rusqlite::params![id.to_string()])?;
        match rows.next()? {
            Some(row) => self.read_message(&db, row),
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
             ORDER BY m.date DESC
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

    fn put_caps(
        &self,
        account: AccountId,
        caps: &AccountCaps,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(account) DO UPDATE SET
                 caps = excluded.caps, observed_at = excluded.observed_at",
            rusqlite::params![
                account.to_string(),
                row::to_json("AccountCaps", caps)?,
                row::from_time(now),
            ],
        )?;
        Ok(())
    }

    fn remotes_of(&self, message: MessageId) -> Result<Vec<RemoteRef>, StoreError> {
        let account = self.message(message)?.account;
        self.refs_for(account, &[message])
    }

    fn hold_part(
        &self,
        message: MessageId,
        section: &str,
        blob: mail_domain::BlobId,
        size: u64,
    ) -> Result<(), StoreError> {
        let mut attachments = self.message(message)?.attachments;
        held(&mut attachments, message, section, blob, size)?;
        self.connection().execute(
            "UPDATE messages SET attachments = ?2 WHERE id = ?1",
            rusqlite::params![
                message.to_string(),
                row::to_json("attachments", &attachments)?
            ],
        )?;
        Ok(())
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

    fn folders(&self, account: AccountId) -> Result<Vec<mail_domain::Folder>, StoreError> {
        self.read_folders(account)
    }

    fn put_folders(
        &self,
        account: AccountId,
        listed: Vec<mail_domain::Folder>,
    ) -> Result<(), StoreError> {
        self.write_folders(account, listed)
    }

    fn folder_contents(
        &self,
        mailbox: &MailboxRef,
    ) -> Result<mail_domain::FolderContents, StoreError> {
        self.contents(mailbox)
    }

    fn outbox_settle(
        &self,
        id: OutboxId,
        settle: Settle,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.settle(id, settle, now)
    }

    fn receipt_answer(
        &self,
        message: MessageId,
    ) -> Result<Option<mail_domain::ReceiptAnswer>, StoreError> {
        self.load_receipt_answer(message)
    }

    fn answer_receipt(
        &self,
        message: MessageId,
        answer: mail_domain::ReceiptAnswer,
        now: DateTime<Utc>,
    ) -> Result<mail_domain::ReceiptAnswer, StoreError> {
        self.write_receipt_answer(message, answer, now)
    }
}
