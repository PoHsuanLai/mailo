//! The database is its owner's alone: the file, and the `-wal` and `-shm` SQLite keeps beside it,
//! are 0600 whatever the umask, and one an earlier version made wider is narrowed when opened.

#![cfg(unix)]

use mail_store::SqliteStore;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_database_and_its_wal_and_shm_are_0600() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mail.db");
    // As an earlier version left it, under a umask that let everyone read it.
    std::fs::write(&db, b"").unwrap();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();

    let _store = SqliteStore::open(&db, dir.path().join("blobs")).unwrap();
    for name in ["mail.db", "mail.db-wal", "mail.db-shm"] {
        let path = dir.path().join(name);
        assert!(path.exists(), "{name} is not there");
        assert_eq!(mode(&path), 0o600, "{name} is {:o}", mode(&path));
    }

    let fresh = dir.path().join("fresh.db");
    let _new = SqliteStore::open(&fresh, dir.path().join("blobs2")).unwrap();
    assert_eq!(mode(&fresh), 0o600, "a new database is {:o}", mode(&fresh));
}
