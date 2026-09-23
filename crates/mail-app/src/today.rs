//! Today: threads opened recently, per Space.
//!
//! Stored in `today.json` under the state directory (`appearance::state_dir`).
//! An entry is a shortcut in the sidebar. Closing it, or letting it expire,
//! removes the shortcut and does not touch the mail.

use crate::appearance::{read_json, write_json};
use chrono::{DateTime, Utc};
use mail_domain::{DraftId, ThreadId};
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "today.json";

/// How long a thread stays in Today after it was last opened.
///
/// Closing or expiring an entry never touches the mail; it only removes the sidebar's shortcut.
pub const IDLE: chrono::Duration = chrono::Duration::hours(12);

/// One opened thread, in one Space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Which Space it was opened in. The same thread in two Spaces is two shortcuts.
    pub space: usize,
    /// The conversation.
    pub thread: ThreadId,
    /// When it was last opened. Opening it again moves this forward.
    pub last_opened: DateTime<Utc>,
}

/// A draft that was put aside with Esc, in one Space. Clicking it opens the composer again.
///
/// Parking is not a timer: a draft stays until it is reopened, sent or discarded, because the
/// draft itself is in the store and this is only the way back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parked {
    /// Which Space it was put aside in.
    pub space: usize,
    /// The draft.
    pub draft: DraftId,
    /// What the entry says: the subject when there is one.
    pub title: String,
    /// When it was parked. The newest is first.
    pub parked: DateTime<Utc>,
}

/// The shortcuts currently in Today, most recently opened first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Today {
    pub entries: Vec<Entry>,
    /// Drafts put aside, newest first. Absent in files written before drafts could be parked.
    pub drafts: Vec<Parked>,
}

impl Today {
    /// Insert `thread` in `space`, or refresh it, and put it first.
    pub fn opened(&mut self, space: usize, thread: ThreadId, now: DateTime<Utc>) {
        self.entries
            .retain(|entry| entry.space != space || entry.thread != thread);
        self.entries.insert(
            0,
            Entry {
                space,
                thread,
                last_opened: now,
            },
        );
    }

    /// Remove the shortcut for `thread` in `space`. The mail stays where it is.
    pub fn close(&mut self, space: usize, thread: ThreadId) {
        self.entries
            .retain(|entry| entry.space != space || entry.thread != thread);
    }

    /// Remove every shortcut in `space`.
    pub fn clear(&mut self, space: usize) {
        self.entries.retain(|entry| entry.space != space);
    }

    /// Shortcuts in `space` that were opened within [`IDLE`], most recent first.
    pub fn live(&self, space: usize, now: DateTime<Utc>) -> Vec<ThreadId> {
        self.entries
            .iter()
            .filter(|entry| entry.space == space && !idle(entry, now))
            .map(|entry| entry.thread)
            .collect()
    }

    /// Drop every shortcut that has been idle for more than [`IDLE`].
    pub fn prune(&mut self, now: DateTime<Utc>) {
        self.entries.retain(|entry| !idle(entry, now));
    }

    /// Put `draft` aside in `space`, or move it to the front with a new title.
    pub fn park(&mut self, space: usize, draft: DraftId, title: &str, now: DateTime<Utc>) {
        self.drafts.retain(|parked| parked.draft != draft);
        self.drafts.insert(
            0,
            Parked {
                space,
                draft,
                title: title.to_owned(),
                parked: now,
            },
        );
    }

    /// The draft is open again, sent, or gone: its entry goes, in every Space.
    pub fn unpark(&mut self, draft: DraftId) {
        self.drafts.retain(|parked| parked.draft != draft);
    }

    /// The drafts parked in `space`, newest first.
    pub fn parked(&self, space: usize) -> Vec<&Parked> {
        self.drafts
            .iter()
            .filter(|parked| parked.space == space)
            .collect()
    }
}

fn idle(entry: &Entry, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(entry.last_opened) > IDLE
}

/// The stored list, or an empty one when there is no file or it cannot be read.
pub fn load(dir: &Path) -> Today {
    read_json(dir, FILE_NAME)
}

/// Write `today` to `dir/today.json`, creating `dir` if needed.
pub fn save(dir: &Path, today: &Today) -> Result<(), String> {
    write_json(dir, FILE_NAME, today)
}

#[cfg(test)]
mod tests {
    use super::{Today, load, save};
    use chrono::{DateTime, Utc};
    use mail_domain::ThreadId;
    use std::path::Path;
    use uuid::Uuid;

    fn thread(n: u128) -> ThreadId {
        ThreadId::from_uuid(Uuid::from_u128(n))
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap_or_else(|| panic!("1_700_000_000 is a valid timestamp"))
            + chrono::Duration::minutes(minutes)
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|e| panic!("{e}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[derive(Clone, Copy)]
    enum Step {
        Open {
            space: usize,
            thread: usize,
            at: i64,
        },
        Close {
            space: usize,
            thread: usize,
        },
        Clear {
            space: usize,
        },
        Live {
            space: usize,
            at: i64,
            expect: &'static [usize],
            name: &'static str,
        },
        FrontAt {
            at: i64,
            name: &'static str,
        },
    }

    #[test]
    fn opening_refreshing_closing_and_clearing_are_per_space() {
        let threads = [thread(1), thread(2), thread(3)];
        let steps = [
            Step::Open {
                space: 0,
                thread: 0,
                at: 0,
            },
            Step::Live {
                space: 0,
                at: 0,
                expect: &[0],
                name: "open lands at the front",
            },
            Step::Open {
                space: 0,
                thread: 1,
                at: 1,
            },
            Step::Live {
                space: 0,
                at: 1,
                expect: &[1, 0],
                name: "a later open is ahead",
            },
            Step::Open {
                space: 0,
                thread: 0,
                at: 2,
            },
            Step::Live {
                space: 0,
                at: 2,
                expect: &[0, 1],
                name: "refresh moves to the front",
            },
            Step::FrontAt {
                at: 2,
                name: "refresh stores the new time",
            },
            Step::Open {
                space: 1,
                thread: 0,
                at: 3,
            },
            Step::Live {
                space: 1,
                at: 3,
                expect: &[0],
                name: "the same thread in another space",
            },
            Step::Live {
                space: 0,
                at: 3,
                expect: &[0, 1],
                name: "the other space does not reorder this one",
            },
            Step::Close {
                space: 0,
                thread: 1,
            },
            Step::Live {
                space: 0,
                at: 3,
                expect: &[0],
                name: "close removes the thread",
            },
            Step::Live {
                space: 1,
                at: 3,
                expect: &[0],
                name: "close leaves the other space",
            },
            Step::Close {
                space: 0,
                thread: 2,
            },
            Step::Live {
                space: 0,
                at: 3,
                expect: &[0],
                name: "closing an absent thread changes nothing",
            },
            Step::Clear { space: 1 },
            Step::Live {
                space: 1,
                at: 3,
                expect: &[],
                name: "clear removes the space",
            },
            Step::Live {
                space: 0,
                at: 3,
                expect: &[0],
                name: "clear leaves the other space",
            },
        ];
        let mut today = Today::default();
        for step in steps {
            match step {
                Step::Open {
                    space,
                    thread,
                    at: minutes,
                } => today.opened(space, threads[thread], at(minutes)),
                Step::Close { space, thread } => today.close(space, threads[thread]),
                Step::Clear { space } => today.clear(space),
                Step::Live {
                    space,
                    at: minutes,
                    expect,
                    name,
                } => {
                    let want: Vec<ThreadId> = expect.iter().map(|index| threads[*index]).collect();
                    assert_eq!(today.live(space, at(minutes)), want, "{name}");
                }
                Step::FrontAt { at: minutes, name } => {
                    let front = today.entries.first().unwrap_or_else(|| panic!("{name}"));
                    assert_eq!(front.last_opened, at(minutes), "{name}");
                }
            }
        }
    }

    #[test]
    fn an_entry_expires_after_twelve_idle_hours() {
        // Space 1 is opened two hours after space 0, so the two lists age apart.
        let rows = [
            ("11h59m", 11 * 60 + 59, true, true),
            ("12h00m", 12 * 60, true, true),
            ("12h01m", 12 * 60 + 1, false, true),
            ("14h01m", 14 * 60 + 1, false, false),
        ];
        let earlier = thread(10);
        let later = thread(11);
        for (name, minutes, space0_kept, space1_kept) in rows {
            let mut today = Today::default();
            today.opened(0, earlier, at(0));
            today.opened(1, later, at(2 * 60));
            let when = at(minutes);
            let live0 = today.live(0, when);
            let live1 = today.live(1, when);
            assert_eq!(today.entries.len(), 2, "{name}: live leaves the record");
            assert_eq!(
                live0,
                if space0_kept { vec![earlier] } else { vec![] },
                "{name} space 0"
            );
            assert_eq!(
                live1,
                if space1_kept { vec![later] } else { vec![] },
                "{name} space 1"
            );
            today.prune(when);
            let mut left: Vec<(usize, ThreadId)> = today
                .entries
                .iter()
                .map(|entry| (entry.space, entry.thread))
                .collect();
            left.sort();
            let mut want = Vec::new();
            if space0_kept {
                want.push((0, earlier));
            }
            if space1_kept {
                want.push((1, later));
            }
            assert_eq!(left, want, "{name}: prune");
        }
    }

    #[test]
    fn today_round_trips() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let fresh = dir.path().join("mailo");
        let mut today = Today::default();
        today.opened(0, thread(4), at(0));
        today.opened(1, thread(5), at(30));
        save(&fresh, &today).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(&fresh), today);
        assert_eq!(entries(&fresh), ["today.json"]);
    }

    #[test]
    fn a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), Today::default());
    }

    #[test]
    fn garbage_bytes_are_empty() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("today.json");
        const CASES: &[(&str, &[u8])] = &[
            ("empty", b""),
            ("prose", b"not json {{{"),
            ("binary", &[0xff, 0xfe, b'{']),
        ];
        for &(name, bytes) in CASES {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(load(dir.path()), Today::default(), "{name}");
        }
    }

    #[test]
    fn a_partial_file_keeps_the_entry_it_has() {
        let id = thread(9);
        let cases = [(
            "an extra field is ignored",
            format!(
                r#"{{"entries":[{{"space":2,"thread":"{id}","last_opened":"2023-11-14T22:13:20Z","future":false}}],"later":1}}"#
            ),
        )];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("today.json");
        for (name, bytes) in cases {
            std::fs::write(&path, bytes.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let loaded = load(dir.path());
            assert_eq!(loaded.entries.len(), 1, "{name}");
            assert_eq!(loaded.entries[0].space, 2, "{name}");
            assert_eq!(loaded.entries[0].thread, id, "{name}");
            assert_eq!(loaded.entries[0].last_opened, at(0), "{name}");
        }
    }
}
