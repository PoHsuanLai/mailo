//! Content-addressed bytes: raw `.eml`, unsanitized HTML, attachment parts.
//!
//! Addressing by hash means the same attachment forwarded through ten threads is stored once,
//! and it means a blob's on-disk path is derived entirely from its content — never from a
//! filename, a `Content-Disposition`, or anything else a stranger wrote.

use crate::StoreError;
use mail_domain::BlobId;
use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::{Path, PathBuf};

/// Bytes at or below this live in SQLite; larger ones go to a file.
///
/// Small parts are mostly `text/plain` alternatives and inline images. Keeping them in the
/// database avoids a syscall per message in a list view, while a 20 MB attachment in a row
/// would make every unrelated query read it.
const INLINE_MAX: usize = 32 * 1024;

/// The blob store rooted at a directory.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// A store under `root`, e.g. `~/.local/share/mailo/blobs`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Store `bytes`, returning the id to reference them by.
    ///
    /// Idempotent: storing identical bytes twice returns the first id and writes nothing, so
    /// re-fetching a message after a `UIDVALIDITY` reset does not duplicate its attachments.
    pub fn put(&self, db: &Connection, bytes: &[u8]) -> Result<BlobId, StoreError> {
        let hash = blake3::hash(bytes).to_hex().to_string();

        if let Some(existing) = self.lookup(db, &hash)? {
            return Ok(existing);
        }

        let id = BlobId::generate();
        let size = bytes.len() as i64;

        if bytes.len() <= INLINE_MAX {
            db.execute(
                "INSERT INTO blobs (id, hash, size, path, inline, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, datetime('now'))",
                params![id.to_string(), hash, size, bytes],
            )
            .map_err(|e| StoreError::Db(e.to_string()))?;
        } else {
            // Two-character fan-out: a flat directory with 100k attachments is slow to list
            // on every filesystem that matters.
            let rel = format!("{}/{}", &hash[..2], &hash[2..]);
            let full = self.root.join(&rel);
            let dir = full.parent().expect("join always yields a parent");
            fs::create_dir_all(dir).map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;
            // Write-then-rename: a crash mid-write must not leave a truncated file sitting at
            // the name its hash promises.
            let tmp = full.with_extension("part");
            fs::write(&tmp, bytes).map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;
            fs::rename(&tmp, &full).map_err(|e| StoreError::Blob(rel.clone(), e.to_string()))?;

            db.execute(
                "INSERT INTO blobs (id, hash, size, path, inline, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, datetime('now'))",
                params![id.to_string(), hash, size, rel],
            )
            .map_err(|e| StoreError::Db(e.to_string()))?;
        }
        Ok(id)
    }

    /// Read the bytes behind `id`.
    pub fn get(&self, db: &Connection, id: BlobId) -> Result<Vec<u8>, StoreError> {
        let row: Option<(Option<String>, Option<Vec<u8>>)> = db
            .query_row(
                "SELECT path, inline FROM blobs WHERE id = ?1",
                params![id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| StoreError::Db(e.to_string()))?;

        match row {
            Some((_, Some(inline))) => Ok(inline),
            Some((Some(rel), None)) => {
                let full = self.resolve(&rel)?;
                fs::read(&full).map_err(|e| StoreError::Blob(rel, e.to_string()))
            }
            // The CHECK constraint forbids it, but a corrupted row must not panic.
            Some((None, None)) => Err(StoreError::Blob(
                id.to_string(),
                "row has neither path nor inline bytes".to_owned(),
            )),
            None => Err(StoreError::Blob(id.to_string(), "no such blob".to_owned())),
        }
    }

    /// How many bytes are behind `id`, without reading them.
    ///
    /// The column has been there since migration 0001 and nothing asked for it, so the one
    /// caller that wanted a size read the whole attachment to measure it — which for a list of
    /// what a draft is carrying means loading every file to print its length.
    pub fn size(&self, db: &Connection, id: BlobId) -> Result<u64, StoreError> {
        let found: Option<i64> = db
            .query_row(
                "SELECT size FROM blobs WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| StoreError::Db(e.to_string()))?;
        found
            .map(|size| size.max(0) as u64)
            .ok_or_else(|| StoreError::Blob(id.to_string(), "no such blob".to_owned()))
    }

    fn lookup(&self, db: &Connection, hash: &str) -> Result<Option<BlobId>, StoreError> {
        let found: Option<String> = db
            .query_row("SELECT id FROM blobs WHERE hash = ?1", params![hash], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| StoreError::Db(e.to_string()))?;
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

    #[test]
    fn a_size_is_answered_without_reading_the_bytes() {
        // The column has been in the schema since migration 0001 and nothing read it, so the
        // one caller that wanted a size read the whole file to measure it. Both sides of the
        // inline threshold, because they are stored differently and only one of them is a file.
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        let db = db();

        let small = store.put(&db, b"hello").unwrap();
        assert_eq!(store.size(&db, small).unwrap(), 5);

        let large = vec![9u8; INLINE_MAX + 100];
        let id = store.put(&db, &large).unwrap();
        assert_eq!(store.size(&db, id).unwrap(), large.len() as u64);
    }

    #[test]
    fn the_size_of_a_blob_that_is_not_there_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        store
            .size(&db(), BlobId::generate())
            .expect_err("no such blob");
    }

    #[test]
    fn small_blobs_round_trip_inline() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        let db = db();
        let id = store.put(&db, b"hello").unwrap();
        assert_eq!(store.get(&db, id).unwrap(), b"hello");
        let path: Option<String> = db
            .query_row(
                "SELECT path FROM blobs WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(path.is_none(), "small blobs must not touch the filesystem");
    }

    #[test]
    fn large_blobs_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        let db = db();
        let big = vec![7u8; INLINE_MAX + 1];
        let id = store.put(&db, &big).unwrap();
        assert_eq!(store.get(&db, id).unwrap(), big);
        let path: Option<String> = db
            .query_row(
                "SELECT path FROM blobs WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(path.is_some(), "large blobs must go to a file");
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
