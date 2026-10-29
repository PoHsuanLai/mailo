//! A calendar that is the person's default calendar application: where there is no calendar store
//! to put an event in (Windows, a desktop with none), the event is written as an `.ics` file and
//! the system is asked to open it, which the application it belongs to turns into its own import.
//!
//! It is a [`CalendarStore`] so the window has one path for every platform, with the limits that
//! say: it has one "calendar" (the application), it cannot find an event it handed over, and it
//! cannot take one back. The opening is a seam ([`Opens`]) so a test never launches anything.

use porter_calendar::{CalendarError, CalendarRef, CalendarStore, Event, Stored};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The id of the one calendar a handoff has.
pub const APPLICATION: &str = "default-application";

/// What opens a file with the application the system keeps for it.
pub trait Opens: Send + Sync {
    /// Ask the system to open `path`. Returns when it has been asked, not when the application
    /// has finished with it.
    fn open(&self, path: &Path) -> Result<(), String>;
}

/// The system's opener: `open`, `xdg-open` or `start`, whichever the platform has.
#[derive(Debug, Clone, Copy)]
pub struct System;

impl Opens for System {
    fn open(&self, path: &Path) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = std::process::Command::new("xdg-open");
        command
            .arg(path)
            .spawn()
            // Reaped by the system when it ends: this process does not wait on an application.
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// The system's opener, to hand to [`Handoff::new`].
pub fn default_application() -> Arc<dyn Opens> {
    Arc::new(System)
}

/// A calendar that is the default calendar application.
#[derive(Clone)]
pub struct Handoff {
    opens: Arc<dyn Opens>,
    dir: PathBuf,
}

impl std::fmt::Debug for Handoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handoff").field("dir", &self.dir).finish()
    }
}

impl Handoff {
    /// A handoff that opens through `opens`, writing its files under the temporary directory.
    pub fn new(opens: Arc<dyn Opens>) -> Self {
        Self::under(opens, std::env::temp_dir().join("mailo-events"))
    }

    /// A handoff that writes its files under `dir`.
    pub fn under(opens: Arc<dyn Opens>, dir: PathBuf) -> Self {
        Self { opens, dir }
    }
}

/// A file name for `uid`: its letters and digits, so no UID can name a path, and a hash of the
/// whole UID, so two UIDs that differ only in what was dropped or cut do not share a file.
fn file_name(uid: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let stem: String = uid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(48)
        .collect();
    // `DefaultHasher::new` is keyed the same every time, so a UID keeps its file name.
    let mut hasher = DefaultHasher::new();
    uid.hash(&mut hasher);
    format!("{stem}-{:016x}.ics", hasher.finish())
}

/// Write `bytes` to a file at `path` that is made new: one already there, a link a stranger left
/// in a shared temporary directory included, is removed rather than written through.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

impl CalendarStore for Handoff {
    async fn calendars(&self) -> Result<Vec<CalendarRef>, CalendarError> {
        Ok(vec![CalendarRef {
            id: APPLICATION.to_owned(),
            name: "Default calendar application".to_owned(),
            color: None,
            writable: true,
        }])
    }

    async fn find(&self, _calendar: &str, _uid: &str) -> Result<Option<Stored>, CalendarError> {
        // An application that was handed a file says nothing back.
        Ok(None)
    }

    async fn put(&self, calendar: &str, event: &Event) -> Result<Stored, CalendarError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| CalendarError::Bus(e.to_string()))?;
        let path = self.dir.join(file_name(&event.uid));
        write_new(&path, event.ics.as_bytes()).map_err(|e| CalendarError::Bus(e.to_string()))?;
        self.opens.open(&path).map_err(CalendarError::Bus)?;
        Ok(Stored {
            calendar: calendar.to_owned(),
            uid: event.uid.clone(),
            id: path.display().to_string(),
            version: None,
            ics: None,
        })
    }

    async fn remove(&self, _calendar: &str, _uid: &str) -> Result<bool, CalendarError> {
        // Nothing here to take back: the application has the event, not this process.
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use porter_calendar::When;
    use std::sync::Mutex;

    /// Remembers what it was asked to open, and opens nothing.
    #[derive(Default)]
    struct Remembers(Mutex<Vec<PathBuf>>);

    impl Opens for Remembers {
        fn open(&self, path: &Path) -> Result<(), String> {
            self.0.lock().unwrap().push(path.to_owned());
            Ok(())
        }
    }

    struct Refuses;

    impl Opens for Refuses {
        fn open(&self, _: &Path) -> Result<(), String> {
            Err("no application".to_owned())
        }
    }

    fn event(uid: &str) -> Event {
        Event {
            uid: uid.to_owned(),
            ics: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_owned(),
            summary: "Review".to_owned(),
            when: When::Timed {
                start: porter_calendar::Instant(1_790_000_000),
                end: porter_calendar::Instant(1_790_003_600),
            },
            location: None,
            description: None,
        }
    }

    #[tokio::test]
    async fn an_event_is_written_as_a_file_named_for_its_uid_and_handed_to_the_opener() {
        let dir = tempfile::tempdir().unwrap();
        let opened = Arc::new(Remembers::default());
        let handoff = Handoff::under(opened.clone(), dir.path().to_owned());
        let calendars = handoff.calendars().await.unwrap();
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].id, APPLICATION);

        // A UID is a stranger's text: it cannot name a path.
        handoff
            .put(APPLICATION, &event("../../etc/x@example.test"))
            .await
            .unwrap();

        let opened = opened.0.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].parent(), Some(dir.path()));
        assert!(opened[0].extension().is_some_and(|e| e == "ics"));
        assert!(
            std::fs::read_to_string(&opened[0])
                .unwrap()
                .starts_with("BEGIN:VCALENDAR")
        );
    }

    #[test]
    fn uids_that_differ_only_in_what_is_dropped_have_their_own_files() {
        assert_ne!(file_name("a@b"), file_name("a-b"));
        assert_eq!(file_name("a@b"), file_name("a@b"));
        let long = "x".repeat(200);
        assert_ne!(
            file_name(&format!("{long}1")),
            file_name(&format!("{long}2"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_left_where_the_file_goes_is_replaced_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "mine").unwrap();
        let path = dir.path().join("event.ics");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        write_new(&path, b"theirs").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
    }

    #[tokio::test]
    async fn it_cannot_find_or_take_back_an_event_and_says_so_when_nothing_opens_it() {
        let dir = tempfile::tempdir().unwrap();
        let handoff = Handoff::under(Arc::new(Refuses), dir.path().to_owned());
        assert_eq!(handoff.find(APPLICATION, "a").await, Ok(None));
        assert_eq!(handoff.remove(APPLICATION, "a").await, Ok(false));
        assert!(handoff.put(APPLICATION, &event("a")).await.is_err());
    }
}
