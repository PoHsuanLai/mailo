//! A saved file, opened in the app the system gives its kind, or shown in its folder.
//!
//! Both block until the system has taken the request, so the window calls them off the thread
//! that draws. What they answer on failure is a sentence the window shows as it is.

use std::path::Path;
use std::process::Command;

/// Open `path` in the app the system associates with its kind.
pub(in crate::ui) fn open(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(gone(path));
    }
    run(opener(path))
}

/// Show `path` in the file manager, selected where the desktop can do that; its folder opened
/// where it cannot.
pub(in crate::ui) fn reveal(path: &Path) -> Result<(), String> {
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

fn gone(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("{name} was moved or deleted.")
}

/// The command that opens `path` in the app the system associates with it.
fn opener(path: &Path) -> Command {
    let (program, lead): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(windows) {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    let mut command = Command::new(program);
    command.args(lead).arg(path);
    command
}

fn run(mut command: Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("Couldn\u{2019}t open it: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Couldn\u{2019}t open it: the opener exited with {status}"
        ))
    }
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
