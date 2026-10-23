//! The mark macOS puts on a file that came from somewhere else, put on every file mailo writes
//! out of a message: an attachment, a saved message, an invitation. Gatekeeper asks before
//! opening a marked program or script, as it does for one a browser downloaded; an unmarked one
//! it opens without a word. Nothing on other systems, which keep no such mark.

use chrono::{DateTime, Utc};
use std::path::Path;

/// The extended attribute macOS reads.
#[cfg(target_os = "macos")]
const ATTRIBUTE: &str = "com.apple.quarantine";

/// The attribute's value: `flags;time;agent;event`. `0081` is a download no one has approved
/// yet, the time is seconds since 1970 in hexadecimal, and the event is left empty.
pub fn value(agent: &str, now: DateTime<Utc>) -> String {
    format!("0081;{:08x};{agent};", now.timestamp().max(0))
}

/// Mark `path` as come from elsewhere, written by `agent` at `now`.
///
/// # Errors
/// When the attribute cannot be set: a file system that keeps no extended attributes.
#[cfg(target_os = "macos")]
pub fn mark(path: &Path, agent: &str, now: DateTime<Utc>) -> std::io::Result<()> {
    xattr::set(path, ATTRIBUTE, value(agent, now).as_bytes())
}

/// Nothing to mark off macOS.
///
/// # Errors
/// Never.
#[cfg(not(target_os = "macos"))]
pub fn mark(_path: &Path, _agent: &str, _now: DateTime<Utc>) -> std::io::Result<()> {
    Ok(())
}

/// [`mark`], at the time it is now.
///
/// # Errors
/// As [`mark`].
pub fn mark_now(path: &Path, agent: &str) -> std::io::Result<()> {
    mark(path, agent, Utc::now())
}

/// The mark on `path`, if it has one.
#[cfg(target_os = "macos")]
pub fn read(path: &Path) -> Option<String> {
    // Unreadable or not text is the same answer as no mark: there is none to go by.
    let bytes = xattr::get(path, ATTRIBUTE).ok()??;
    String::from_utf8(bytes).ok() // As above.
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_value_is_an_unapproved_download_at_its_time_by_its_agent() {
        let at = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        assert_eq!(
            value("mailo", at),
            format!("0081;{:08x};mailo;", at.timestamp())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_marked_file_reads_back_its_mark() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.command");
        std::fs::write(&file, b"echo hi").unwrap();
        assert_eq!(read(&file), None);
        let at = Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap();
        mark(&file, "mailo", at).unwrap();
        assert_eq!(read(&file), Some(value("mailo", at)));
    }
}
