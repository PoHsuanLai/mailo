//! Content-addressed bytes: raw `.eml`, unsanitized HTML, attachment parts.
//!
//! Addressing by hash means the same attachment forwarded through ten threads is stored once,
//! and it means a blob's on-disk path is derived entirely from its content — never from a
//! filename, a `Content-Disposition`, or anything else a stranger wrote.

use crate::{SqliteStore, StoreError};
use mail_domain::BlobId;
use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::{Path, PathBuf};

/// The blobs of one [`SqliteStore`]: content-addressed bytes (a raw message, an attachment)
/// that the store keeps in its database or on disk, whichever suits their size.
///
/// A handle on the store and nothing else, so a caller names a blob by its id and never by the
/// connection it is read through: which connection that is, and whether the bytes are in a row
/// or a file, is the store's.
#[derive(Debug, Clone, Copy)]
pub struct Blobs<'a> {
    store: &'a SqliteStore,
}

impl<'a> Blobs<'a> {
    pub(crate) fn of(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Store `bytes`, returning the id to reference them by. Identical bytes are stored once
    /// and share an id.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId, StoreError> {
        // The file, if there is one, is written before the writer is taken: its fsync is the
        // slow part, and nobody else's write should wait on it.
        let files = self.store.files();
        let staged = files.stage(bytes)?;
        // A body or an attachment: what a message shows.
        files.record(&self.store.touch(crate::Topics::SUMMARIES), &staged, bytes)
    }

    /// The bytes behind `id`.
    pub fn get(&self, id: BlobId) -> Result<Vec<u8>, StoreError> {
        self.store.files().get(&self.store.reader(), id)
    }

    /// At most the first `limit` bytes behind `id`: enough of a raw message for its headers,
    /// without reading a large attachment to find them.
    pub fn head(&self, id: BlobId, limit: usize) -> Result<Vec<u8>, StoreError> {
        self.store.files().head(&self.store.reader(), id, limit)
    }

    /// How many bytes are behind `id`, without reading them.
    pub fn size(&self, id: BlobId) -> Result<u64, StoreError> {
        self.store.files().size(&self.store.reader(), id)
    }
}

/// Bytes at or below this live in SQLite; larger ones go to a file.
///
/// Small parts are mostly `text/plain` alternatives and inline images. Keeping them in the
/// database avoids a syscall per message in a list view, while a 20 MB attachment in a row
/// would make every unrelated query read it.
const INLINE_MAX: usize = 32 * 1024;

/// How long a blob or its file may go unused before a put of the same bytes says so again. Well
/// under the sweep's grace period, which is how long a sweep leaves an unused one alone.
const REFRESH_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const REFRESH_AFTER_SQL: &str = "-30 minutes";

/// Say that the file at `path` is in use now, if it has not been said for a while.
fn refresh_file(path: &Path) {
    let old = fs::metadata(path)
        .and_then(|meta| meta.modified())
        .is_ok_and(|at| at.elapsed().is_ok_and(|age| age > REFRESH_AFTER));
    if old && let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        // A failure leaves the old time, which at worst lets a sweep take the file and the next
        // put write it again.
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Where a blob's bytes are: in its row, or in the file at a path relative to the root.
enum Found {
    Inline(Vec<u8>),
    File(String),
}

/// A blob whose file, if it has one, is in place and whose row is not yet written.
pub(crate) struct Staged {
    hash: String,
    /// Where its file is, relative to the root; `None` for one small enough to live in its row.
    path: Option<String>,
    size: i64,
}

/// The blob store rooted at a directory.
#[derive(Debug, Clone)]
pub(crate) struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// A store under `root`, e.g. `~/.local/share/mailo/blobs`.
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Put the file for a large blob in place, if it is not already, and return what
    /// [`BlobStore::record`] needs. Takes no lock: an fsync of a large attachment must not hold
    /// up every other writer.
    ///
    /// The file is looked for whether or not a row names it, so a file that went missing is
    /// written again by the next put of the same bytes instead of staying lost. Two puts of the
    /// same bytes at once may both write: each writes its own temporary file and renames it onto
    /// the same name, and a rename replaces atomically with identical content, so whichever lands
    /// last leaves the same file and a reader never sees a half-written one.
    pub(crate) fn stage(&self, bytes: &[u8]) -> Result<Staged, StoreError> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        let path = if bytes.len() <= INLINE_MAX {
            None
        } else {
            // Two-character fan-out: a flat directory with 100k attachments is slow to list
            // on every filesystem that matters.
            let rel = format!("{}/{}", &hash[..2], &hash[2..]);
            let full = self.root.join(&rel);
            if full.is_file() {
                // A put of bytes already kept says the file is in use, which is what keeps a
                // sweep from taking it as an orphan between here and the row being written.
                refresh_file(&full);
            } else {
                let dir = full.parent().expect("join always yields a parent");
                fs::create_dir_all(dir)
                    .map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;
                // Write-then-rename: a crash mid-write must not leave a truncated file sitting
                // at the name its hash promises. Its owner's alone, as the message it came out
                // of is.
                crate::file::write_atomic(&full, bytes, Some(0o600))
                    .map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;
            }
            Some(rel)
        };
        Ok(Staged {
            hash,
            path,
            size: bytes.len() as i64,
        })
    }

    /// Record a staged blob's row, returning the id to reference it by. Short: it is the part
    /// that needs the writer.
    ///
    /// Idempotent: bytes already recorded return the first id and write nothing, so re-fetching
    /// a message after a `UIDVALIDITY` reset does not duplicate its attachments.
    pub(crate) fn record(
        &self,
        db: &Connection,
        staged: &Staged,
        bytes: &[u8],
    ) -> Result<BlobId, StoreError> {
        if let Some(existing) = self.lookup(db, &staged.hash)? {
            // Put again, so it is in use now: a sweep takes only a row that has gone unused for
            // a while, and without this one put again just as the sweep decides would lose it.
            // Only when it is getting old, so the usual put of bytes already kept writes nothing.
            db.prepare_cached(
                "UPDATE blobs SET created_at = datetime('now')
                 WHERE id = ?1 AND created_at < datetime('now', ?2)",
            )
            .map_err(StoreError::db)?
            .execute(params![existing.to_string(), REFRESH_AFTER_SQL])
            .map_err(StoreError::db)?;
            return Ok(existing);
        }
        let id = BlobId::generate();
        match &staged.path {
            None => db
                .prepare_cached(
                    "INSERT INTO blobs (id, hash, size, path, inline, created_at)
                     VALUES (?1, ?2, ?3, NULL, ?4, datetime('now'))",
                )
                .map_err(StoreError::db)?
                .execute(params![id.to_string(), staged.hash, staged.size, bytes]),
            Some(rel) => db
                .prepare_cached(
                    "INSERT INTO blobs (id, hash, size, path, inline, created_at)
                     VALUES (?1, ?2, ?3, ?4, NULL, datetime('now'))",
                )
                .map_err(StoreError::db)?
                .execute(params![id.to_string(), staged.hash, staged.size, rel]),
        }
        .map_err(StoreError::db)?;
        Ok(id)
    }

    /// Stage and record in one go, for a test that has a connection and no store.
    #[cfg(test)]
    fn put(&self, db: &Connection, bytes: &[u8]) -> Result<BlobId, StoreError> {
        let staged = self.stage(bytes)?;
        self.record(db, &staged, bytes)
    }

    /// Read the bytes behind `id`.
    pub(crate) fn get(&self, db: &Connection, id: BlobId) -> Result<Vec<u8>, StoreError> {
        match self.find(db, id, None)? {
            Found::Inline(inline) => Ok(inline),
            Found::File(rel) => {
                let full = self.resolve(&rel)?;
                fs::read(&full).map_err(|e| StoreError::Blob(rel, e.to_string()))
            }
        }
    }

    /// At most the first `limit` bytes behind `id`: enough of a raw message for its headers,
    /// without reading a large attachment to find them.
    pub(crate) fn head(
        &self,
        db: &Connection,
        id: BlobId,
        limit: usize,
    ) -> Result<Vec<u8>, StoreError> {
        match self.find(db, id, Some(limit))? {
            Found::Inline(inline) => Ok(inline),
            Found::File(rel) => {
                let full = self.resolve(&rel)?;
                let file = fs::File::open(&full)
                    .map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;
                let mut head = Vec::with_capacity(limit);
                std::io::Read::read_to_end(&mut std::io::Read::take(file, limit as u64), &mut head)
                    .map_err(|e| StoreError::Blob(rel, e.to_string()))?;
                Ok(head)
            }
        }
    }

    /// Where the bytes behind `id` are, with at most the first `limit` of them when they are in
    /// the row.
    fn find(&self, db: &Connection, id: BlobId, limit: Option<usize>) -> Result<Found, StoreError> {
        let pair = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?));
        let key = id.to_string();
        let row: Option<(Option<String>, Option<Vec<u8>>)> = match limit {
            None => db
                .prepare_cached("SELECT path, inline FROM blobs WHERE id = ?1")
                .map_err(StoreError::db)?
                .query_row(params![key], pair),
            Some(limit) => db
                .prepare_cached("SELECT path, substr(inline, 1, ?2) FROM blobs WHERE id = ?1")
                .map_err(StoreError::db)?
                .query_row(params![key, limit as i64], pair),
        }
        .optional()
        .map_err(StoreError::db)?;
        match row {
            Some((_, Some(inline))) => Ok(Found::Inline(inline)),
            Some((Some(rel), None)) => Ok(Found::File(rel)),
            // The CHECK constraint forbids it, but a corrupted row must not panic.
            Some((None, None)) => Err(StoreError::Blob(
                key,
                "row has neither path nor inline bytes".to_owned(),
            )),
            None => Err(StoreError::Blob(key, "no such blob".to_owned())),
        }
    }

    /// How many bytes are behind `id`, without reading them.
    ///
    /// The column has been there since migration 0001 and nothing asked for it, so the one
    /// caller that wanted a size read the whole attachment to measure it — which for a list of
    /// what a draft is carrying means loading every file to print its length.
    pub(crate) fn size(&self, db: &Connection, id: BlobId) -> Result<u64, StoreError> {
        let found: Option<i64> = db
            .query_row(
                "SELECT size FROM blobs WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(StoreError::db)?;
        found
            .map(|size| size.max(0) as u64)
            .ok_or_else(|| StoreError::Blob(id.to_string(), "no such blob".to_owned()))
    }

    fn lookup(&self, db: &Connection, hash: &str) -> Result<Option<BlobId>, StoreError> {
        let found: Option<String> = db
            .prepare_cached("SELECT id FROM blobs WHERE hash = ?1")
            .map_err(StoreError::db)?
            .query_row(params![hash], |r| r.get(0))
            .optional()
            .map_err(StoreError::db)?;
        found
            .map(|text| {
                text.parse::<uuid::Uuid>()
                    .map(BlobId::from_uuid)
                    .map_err(|e| StoreError::Decode {
                        what: "BlobId".to_owned(),
                        why: e.to_string(),
                    })
            })
            .transpose()
    }

    /// Move the file at `rel` aside, where nothing reads it, until [`BlobStore::drop_aside`]
    /// deletes it or [`BlobStore::bring_back`] restores it. Already gone is the state wanted.
    /// Refuses a path that is not hash-shaped, as reading does.
    pub(crate) fn set_aside(&self, rel: &str) -> Result<(), StoreError> {
        let path = self.resolve(rel)?;
        match fs::rename(&path, path.with_extension("aside")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Blob(rel.to_owned(), e.to_string())),
        }
    }

    /// Delete the file [`BlobStore::set_aside`] moved aside.
    pub(crate) fn drop_aside(&self, rel: &str) -> Result<(), StoreError> {
        let aside = self.resolve(rel)?.with_extension("aside");
        match fs::remove_file(&aside) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Blob(rel.to_owned(), e.to_string())),
        }
    }

    /// Put the file [`BlobStore::set_aside`] moved aside back where it was.
    pub(crate) fn bring_back(&self, rel: &str) -> Result<(), StoreError> {
        let path = self.resolve(rel)?;
        fs::rename(path.with_extension("aside"), &path)
            .map_err(|e| StoreError::Blob(rel.to_owned(), e.to_string()))
    }

    /// Where the blob root is, which is also where a marker of the store's own can be kept.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Every file in the two-character directories under the root, as a path relative to it and
    /// when it was last written. Includes what is not a blob's file (a half-written `.part`, a
    /// set-aside `.aside`): telling them apart is the caller's. Nothing for a root not made yet.
    pub(crate) fn stored_files(&self) -> Result<Vec<(String, std::time::SystemTime)>, StoreError> {
        let mut found = Vec::new();
        let fans = match fs::read_dir(&self.root) {
            Ok(fans) => fans,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(found),
            Err(e) => {
                return Err(StoreError::Blob(
                    self.root.display().to_string(),
                    e.to_string(),
                ));
            }
        };
        for fan in fans.flatten() {
            let Some(dir) = fan.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if dir.len() != 2 || !dir.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let files = match fs::read_dir(fan.path()) {
                Ok(files) => files,
                // Gone since the listing above: nothing in it to find.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(StoreError::Blob(dir, e.to_string())),
            };
            for file in files.flatten() {
                let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                // Unreadable now is gone or not ours; either way not something to sweep.
                let Ok(meta) = file.metadata() else { continue };
                let Ok(modified) = meta.modified() else {
                    continue;
                };
                if meta.is_file() {
                    found.push((format!("{dir}/{name}"), modified));
                }
            }
        }
        Ok(found)
    }

    /// Delete the file at `rel`, a path [`BlobStore::stored_files`] gave. Already gone is the
    /// state wanted.
    pub(crate) fn remove_stored(&self, rel: &str) -> Result<(), StoreError> {
        match fs::remove_file(self.root.join(rel)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Blob(rel.to_owned(), e.to_string())),
        }
    }

    /// Join a stored relative path to the root, refusing anything that escapes it.
    ///
    /// Stored paths are hex we generated, so this should be unreachable — which is exactly why
    /// it is checked. A path that escapes the blob root would turn a corrupted row into an
    /// arbitrary file read.
    fn resolve(&self, rel: &str) -> Result<PathBuf, StoreError> {
        let path = Path::new(rel);
        let safe = path.components().all(|c| {
            matches!(c, std::path::Component::Normal(part)
                if part.to_str().is_some_and(|s| s.chars().all(|c| c.is_ascii_hexdigit())))
        });
        if !safe {
            return Err(StoreError::Blob(
                rel.to_owned(),
                "stored path is not a hash-shaped relative path".to_owned(),
            ));
        }
        Ok(self.root.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::migrate::migrate(&db).unwrap();
        db
    }

    /// Small blobs are kept in their row and large ones in a file, and each reads back whole with
    /// its size answered from the row. The size column was in the schema since migration 0001
    /// and nothing read it, so the one caller that wanted a size read the whole file to measure
    /// it.
    #[test]
    fn blobs_round_trip_inline_when_small_and_on_disk_when_large() {
        let small = b"hello".to_vec();
        let large = vec![7u8; INLINE_MAX + 1];
        for (bytes, on_disk) in [(small, false), (large, true)] {
            let dir = tempfile::tempdir().unwrap();
            let store = BlobStore::new(dir.path());
            let db = db();
            let id = store.put(&db, &bytes).unwrap();
            assert_eq!(store.get(&db, id).unwrap(), bytes);
            assert_eq!(store.size(&db, id).unwrap(), bytes.len() as u64);
            let path: Option<String> = db
                .query_row(
                    "SELECT path FROM blobs WHERE id = ?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(path.is_some(), on_disk, "{} bytes", bytes.len());
            // A blob written as a file is its owner's alone, as the message it came out of is.
            #[cfg(unix)]
            if let Some(rel) = &path {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = std::fs::metadata(dir.path().join(rel))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "{mode:o}");
            }
            // No stray .part file survived the write-then-rename.
            let parts: Vec<_> = walk(dir.path())
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "part"))
                .collect();
            assert!(
                parts.is_empty(),
                "temporary files must be renamed away: {parts:?}"
            );
        }
    }

    #[test]
    fn identical_bytes_are_stored_once() {
        // The dedup that makes re-fetching after a UIDVALIDITY reset cheap instead of
        // duplicating every attachment.
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        let db = db();
        let a = store.put(&db, b"same").unwrap();
        let b = store.put(&db, b"same").unwrap();
        assert_eq!(a, b);
        let rows: i64 = db
            .query_row("SELECT count(*) FROM blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_path_escaping_the_root_is_refused() {
        // Unreachable through `put`, which writes hex. Checked because a corrupted row would
        // otherwise turn into an arbitrary file read.
        let store = BlobStore::new("/tmp/blobroot");
        for evil in ["../../etc/passwd", "ab/../../x", "/etc/passwd", "ab/cd;rm"] {
            assert!(store.resolve(evil).is_err(), "must refuse {evil}");
        }
        assert!(store.resolve("ab/cdef01").is_ok());
    }

    #[test]
    fn a_missing_blob_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        assert!(store.get(&db(), BlobId::generate()).is_err());
        store
            .size(&db(), BlobId::generate())
            .expect_err("no such blob");
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p)
                } else {
                    out.push(p)
                }
            }
        }
        out
    }
}
