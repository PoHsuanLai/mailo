//! Writing a file so that a crash leaves the old one or the new one, never half of either.

use std::io::Write as _;
use std::path::Path;

/// Write `bytes` to `path` through a temporary file beside it and a rename.
///
/// The temporary sits in the same directory because a rename across filesystems is a copy, and
/// a copy is the half-written file again. A failure removes it. `mode` is the Unix permission
/// the file is created with, set before any byte is written so a secret is never readable even
/// for a moment; `None` leaves the default, and it means nothing off Unix.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        // A path with no file name has its part file named ".part" beside it.
        .unwrap_or_default();
    let part = path.with_file_name(format!(".{name}.part"));
    let written = create(&part, mode).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    written
        .and_then(|()| std::fs::rename(&part, path))
        .inspect_err(|_| {
            // Best-effort cleanup: the write's own error is what is reported.
            let _ = std::fs::remove_file(&part);
        })
}

fn create(path: &Path, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    // No permission bits to set off Unix.
    let _ = mode;
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_replaced_whole_and_no_temporary_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.json");
        write_atomic(&path, b"one", None).unwrap();
        write_atomic(&path, b"two", None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_is_the_files_from_its_first_byte() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_atomic(&path, b"x", Some(0o600)).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
