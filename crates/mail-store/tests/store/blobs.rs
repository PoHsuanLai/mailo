//! Large blobs are files named by their hash: written without the database's lock held, and
//! written again when one has gone missing.

use mail_store::SqliteStore;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Bigger than what a blob row holds, so the bytes go to a file.
fn large(seed: u8) -> Vec<u8> {
    vec![seed; 64 * 1024]
}

fn opened(dir: &Path) -> SqliteStore {
    SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(fans) = std::fs::read_dir(dir.join("blobs")) else {
        return found;
    };
    for fan in fans.flatten() {
        for file in std::fs::read_dir(fan.path()).unwrap().flatten() {
            found.push(file.path());
        }
    }
    found
}

fn rows(store: &SqliteStore) -> i64 {
    store
        .raw_connection()
        .query_row("SELECT count(*) FROM blobs", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_same_bytes_twice_are_one_row_and_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let bytes = large(1);
    let first = store.blobs().put(&bytes).unwrap();
    let second = store.blobs().put(&bytes).unwrap();
    assert_eq!(first, second);
    assert_eq!(rows(&store), 1);
    assert_eq!(files(dir.path()).len(), 1);
    assert_eq!(store.blobs().get(first).unwrap(), bytes);
}

#[test]
fn a_file_that_went_missing_is_written_again_by_the_next_put() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let bytes = large(2);
    let id = store.blobs().put(&bytes).unwrap();
    for file in files(dir.path()) {
        std::fs::remove_file(file).unwrap();
    }
    store.blobs().get(id).expect_err("the file is gone");

    assert_eq!(store.blobs().put(&bytes).unwrap(), id);
    assert_eq!(store.blobs().get(id).unwrap(), bytes);
    assert_eq!(rows(&store), 1);
}

#[test]
fn a_large_blobs_file_is_written_while_another_writer_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let bytes = large(3);
    let held = store.raw_connection();
    std::thread::scope(|scope| {
        let putting = scope.spawn(|| store.blobs().put(&bytes).unwrap());
        // The put cannot finish while the writer is held, but its file does not need the writer.
        let deadline = Instant::now() + Duration::from_secs(10);
        while files(dir.path()).is_empty() {
            assert!(Instant::now() < deadline, "the file waited for the writer");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!putting.is_finished());
        drop(held);
        putting.join().unwrap();
    });
    assert_eq!(rows(&store), 1);
}
