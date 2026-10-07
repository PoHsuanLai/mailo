//! The files the window has saved, newest first: what the Downloads list draws.
//!
//! Stored in `downloads.json` under the state directory. The list is only a way back to each
//! file; removing an entry, or clearing the list, never touches the file itself.

use chrono::{DateTime, Utc};
use mail_core::config::{read_json, write_json};
use mail_domain::ThreadId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "downloads.json";

/// How many files the list remembers; the oldest goes first.
pub(in crate::ui) const KEPT: usize = 50;

/// The conversation a file came out of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::ui) struct Origin {
    pub thread: ThreadId,
    /// Its subject when the file was saved, so the list need not read the store to draw.
    pub subject: String,
}

/// One saved file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::ui) struct Entry {
    /// Where it was written.
    pub path: PathBuf,
    /// When.
    pub at: DateTime<Utc>,
    /// How big it was then.
    pub bytes: u64,
    /// The conversation it came out of; `None` for a file of the window's own, such as an export.
    #[serde(default)]
    pub origin: Option<Origin>,
}

impl Entry {
    /// The name the list shows: the file's own.
    pub(in crate::ui) fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

/// Every saved file the list remembers, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::ui) struct Log {
    pub entries: Vec<Entry>,
}

impl Log {
    /// Put `entry` first. A file saved again under the same path is one entry, not two; past
    /// [`KEPT`] the oldest is forgotten.
    pub(in crate::ui) fn record(&mut self, entry: Entry) {
        self.entries.retain(|kept| kept.path != entry.path);
        self.entries.insert(0, entry);
        self.entries.truncate(KEPT);
    }

    /// Forget the entry for `path`; the file stays where it is.
    pub(in crate::ui) fn remove(&mut self, path: &Path) {
        self.entries.retain(|kept| kept.path != path);
    }
}

/// The stored list, or an empty one when there is no file or it cannot be read.
pub(in crate::ui) fn load(dir: &Path) -> Log {
    read_json(dir, FILE_NAME)
}

/// Write `log` to `dir/downloads.json`, creating `dir` if needed.
pub(in crate::ui) fn save(dir: &Path, log: &Log) -> Result<(), String> {
    write_json(dir, FILE_NAME, log)
}

#[cfg(test)]
mod tests {
    use super::{Entry, KEPT, Log, load, save};
    use chrono::DateTime;
    use std::path::PathBuf;

    fn entry(name: &str, seconds: i64) -> Entry {
        Entry {
            path: PathBuf::from("/saves").join(name),
            at: DateTime::from_timestamp(1_700_000_000 + seconds, 0)
                .unwrap_or_else(|| panic!("a valid timestamp")),
            bytes: 10,
            origin: None,
        }
    }

    fn names(log: &Log) -> Vec<String> {
        log.entries.iter().map(Entry::name).collect()
    }

    #[test]
    fn the_newest_file_comes_first_and_a_path_saved_again_is_one_entry() {
        let mut log = Log::default();
        log.record(entry("a.pdf", 0));
        log.record(entry("b.pdf", 1));
        log.record(entry("a.pdf", 2));
        assert_eq!(names(&log), ["a.pdf", "b.pdf"]);
        assert_eq!(log.entries[0].at, entry("a.pdf", 2).at);
    }

    #[test]
    fn past_the_kept_number_the_oldest_is_forgotten() {
        let mut log = Log::default();
        for n in 0..=KEPT {
            log.record(entry(&format!("{n}.txt"), i64::try_from(n).unwrap_or(0)));
        }
        assert_eq!(log.entries.len(), KEPT);
        assert_eq!(log.entries[0].name(), format!("{KEPT}.txt"));
        assert!(!names(&log).contains(&"0.txt".to_owned()));
    }

    #[test]
    fn removing_an_entry_leaves_the_others() {
        let mut log = Log::default();
        log.record(entry("a.pdf", 0));
        log.record(entry("b.pdf", 1));
        log.remove(&PathBuf::from("/saves/b.pdf"));
        assert_eq!(names(&log), ["a.pdf"]);
    }

    #[test]
    fn the_list_reads_back_what_was_written() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), Log::default());
        let mut log = Log::default();
        log.record(entry("a.pdf", 0));
        save(dir.path(), &log).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), log);
    }
}
