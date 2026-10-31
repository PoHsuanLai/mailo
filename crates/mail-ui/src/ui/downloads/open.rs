//! A saved file, opened in the app the system gives its kind, or shown in its folder. On the
//! quire desktop a file opens in anyview, the desktop's viewer, which shows nearly every kind an
//! attachment comes in. Elsewhere, or when anyview does not answer, the desktop's own
//! open-a-file service hands it to the app the person chose for its kind; the system's opener
//! program is the way when no such service runs.
//!
//! A program or a script is never opened from here: opening one runs it, so it is shown in its
//! folder instead, and the person can open it from there knowing what it is.
//!
//! Both block until the system has taken the request, so the window calls them off the thread
//! that draws. What they answer on failure is a [`NotOpened`]: a sentence the window shows as it
//! is, or a library's words it says plainly ([`NotOpened::told`]).

use mail_ui_model::copy::errors::{couldnt_open, moved_or_deleted};
use std::path::Path;
use std::process::Command;

/// Why a file was not opened or shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotOpened {
    /// A sentence for the person, shown as it is: the file was moved or deleted, a program was
    /// shown in its folder instead, the opener said why it would not.
    Said(String),
    /// A library's or the system's own words, for the log: the window says its own sentence.
    Failed(String),
}

impl NotOpened {
    /// What the window says: the sentence, or `said` with the library's words in the log.
    #[track_caller]
    pub(in crate::ui) fn told(self, said: &str) -> String {
        match self {
            NotOpened::Said(sentence) => sentence,
            NotOpened::Failed(why) => crate::ui::remedy::plainly(said, why),
        }
    }
}

/// Open `path` in the app the system associates with its kind.
pub(in crate::ui) fn open(path: &Path) -> Result<(), NotOpened> {
    opened(path, reveal, launch)
}

/// [`open`], with what shows a file in its folder and what opens it handed in.
fn opened(
    path: &Path,
    reveal: impl FnOnce(&Path) -> Result<(), NotOpened>,
    launch: impl FnOnce(&Path) -> Result<(), NotOpened>,
) -> Result<(), NotOpened> {
    if !path.exists() {
        return Err(gone(path));
    }
    if let Some(name) = runs_when_opened(path) {
        reveal(path)?;
        return Err(NotOpened::Said(mail_ui_model::copy::toasts::shown_not_run(
            &name,
        )));
    }
    launch(path)
}

/// `path` opened in its app: anyview, else the desktop's service, else the system's opener.
fn launch(path: &Path) -> Result<(), NotOpened> {
    #[cfg(all(feature = "quire-desktop", not(any(target_os = "macos", windows))))]
    if in_anyview(path).is_ok() {
        return Ok(());
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    if by_service(path).is_ok() {
        return Ok(());
    }
    run(opener(path))
}

/// `path`'s name, when it names a program or a script, which [`open`] shows in its folder
/// rather than runs.
fn runs_when_opened(path: &Path) -> Option<String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    name.filter(|name| mail_client::attach::runs_when_opened(name))
}

/// Show `path` in the file manager, selected where the desktop can do that; its folder opened
/// where it cannot.
pub(in crate::ui) fn reveal(path: &Path) -> Result<(), NotOpened> {
    if !path.exists() {
        return Err(gone(path));
    }
    #[cfg(target_os = "macos")]
    return run({
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    });
    #[cfg(windows)]
    return run({
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    });
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if show_items(path).is_ok() {
            return Ok(());
        }
        let folder = path.parent().unwrap_or(path);
        run(opener(folder))
    }
}

fn gone(path: &Path) -> NotOpened {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    NotOpened::Said(moved_or_deleted(&name))
}

/// The command that opens `path` in the app the system associates with it. Never a shell: on
/// Windows `cmd /C start` would read a `&` in the file's name as the start of another command.
fn opener(path: &Path) -> Command {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(program);
    command.arg(path);
    command
}

/// Run the opener. That it could not be started is the system's words; that it ran and said no
/// is the person's sentence.
fn run(mut command: Command) -> Result<(), NotOpened> {
    // Explorer exits 1 whether or not it opened anything, so on Windows only starting it can fail.
    if cfg!(windows) {
        return command
            .spawn()
            .map(|_| ())
            .map_err(|e| NotOpened::Failed(e.to_string()));
    }
    let status = command
        .status()
        .map_err(|e| NotOpened::Failed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(NotOpened::Said(couldnt_open(&format!(
            "the opener exited with {status}"
        ))))
    }
}

/// The desktop's open-a-file service on the session bus (`org.freedesktop.portal.Desktop`,
/// `OpenFile`). It is given the open file rather than a path, so it works for an app that is
/// sandboxed from the folder the file is in, and it answers an error when no such service runs.
#[cfg(not(any(target_os = "macos", windows)))]
fn by_service(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let fd = zbus::zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(file));
    let options = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    let connection = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.OpenURI"),
            "OpenFile",
            &("", fd, options),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// anyview's `Open` on the session bus (`org.quire.Anyview1`, which starts anyview when it is not
/// running): an array of absolute paths to files that exist.
#[cfg(all(feature = "quire-desktop", not(any(target_os = "macos", windows))))]
fn in_anyview(path: &Path) -> Result<(), String> {
    let full = path.canonicalize().map_err(|e| e.to_string())?;
    let full = full
        .to_str()
        .ok_or_else(|| format!("{} is not a path anyview can be given", full.display()))?;
    let connection = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    connection
        .call_method(
            Some("org.quire.Anyview1"),
            "/org/quire/Anyview1",
            Some("org.quire.Anyview1"),
            "Open",
            &(vec![full],),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The file manager's own "show this file" on the session bus (`org.freedesktop.FileManager1`),
/// which GNOME Files, Dolphin and most others answer.
#[cfg(not(any(target_os = "macos", windows)))]
fn show_items(path: &Path) -> Result<(), String> {
    let uri = url::Url::from_file_path(path)
        .map_err(|()| format!("{} is not a full path", path.display()))?;
    let connection = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    connection
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![uri.as_str()], ""),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{NotOpened, opened, opener};
    use mail_ui_model::copy::errors::FILE_NOT_OPENED;
    use std::path::Path;

    fn never(_: &Path) -> Result<(), NotOpened> {
        panic!("nothing is launched for this file")
    }

    /// The sentences `open` writes for the person reach the toast as they are; only a library's
    /// words become the window's own sentence.
    #[test]
    fn what_open_says_for_the_person_reaches_the_toast_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("plans.pdf");
        let said = opened(&missing, never, never)
            .unwrap_err()
            .told(FILE_NOT_OPENED);
        assert_eq!(said, "plans.pdf was moved or deleted.");

        let program = dir.path().join("setup.exe");
        std::fs::write(&program, b"MZ").unwrap();
        let said = opened(&program, |_| Ok(()), never)
            .unwrap_err()
            .told(FILE_NOT_OPENED);
        assert_eq!(
            said,
            mail_ui_model::copy::toasts::shown_not_run("setup.exe")
        );
        assert!(said.contains("shown in its folder"), "{said}");

        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"hi").unwrap();
        let refused = |_: &Path| {
            Err(NotOpened::Said(mail_ui_model::copy::errors::couldnt_open(
                "the opener exited with exit status: 4",
            )))
        };
        let said = opened(&file, never, refused)
            .unwrap_err()
            .told(FILE_NOT_OPENED);
        assert!(
            said.contains("the opener exited with exit status: 4"),
            "{said}"
        );

        let broke = |_: &Path| {
            Err(NotOpened::Failed(
                "No such file or directory (os error 2)".into(),
            ))
        };
        let said = opened(&file, never, broke)
            .unwrap_err()
            .told(FILE_NOT_OPENED);
        assert_eq!(said, FILE_NOT_OPENED);
    }

    /// An opener that runs and says no is told as its exit; one that cannot start is the system's.
    #[cfg(unix)]
    #[test]
    fn the_openers_exit_is_a_sentence_and_its_failure_to_start_is_not() {
        let said = super::run(std::process::Command::new("false")).unwrap_err();
        assert!(
            matches!(&said, NotOpened::Said(sentence) if sentence.contains("the opener exited with")),
            "{said:?}"
        );
        let missing = super::run(std::process::Command::new("/nonexistent/mailo-opener"));
        assert!(matches!(missing, Err(NotOpened::Failed(_))), "{missing:?}");
    }

    #[test]
    fn the_opener_is_given_the_path_as_one_argument_and_no_shell() {
        let path = Path::new("/tmp/R&D x&calc.exe");
        let command = opener(path);
        assert_ne!(command.get_program(), "cmd");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [path.as_os_str()]);
    }
}
