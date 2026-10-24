//! "Check for mail when quit" (`sync.while_quit`): `mailo watch` started at each login, so mail is
//! checked, and new mail said, while mailo is not running.
//!
//! Where the system keeps such a thing is a seam ([`LoginItem`]): the platform's own in the
//! launched app ([`platform`]), a recorder in the tests, which must never register a login item,
//! run `launchctl` or write the registry. The Settings sheet hands each change of the switch to
//! it ([`apply`]), and the launched app hands it the switch as it stands once at each start, so a
//! login item made by a mailo since moved elsewhere names the program where it is now.
//!
//! What each platform keeps:
//! - macOS: a launch agent. On macOS 13 and later, `SMAppService` over the property list in
//!   mailo.app's own bundle (`mail_appkit::login`, `packaging/macos/bundle.sh`), which lists
//!   mailo under System Settings › General › Login Items; before 13, or for a mailo run from
//!   outside its bundle, the same agent written to `~/Library/LaunchAgents` and loaded with
//!   `launchctl` ([`launch_agent`]).
//! - Linux and the BSDs: the systemd user unit the packages install (`dist/mailo-watch.service`),
//!   enabled and started for this user with `systemctl --user`. A unit `dist/install.sh` enabled
//!   for every user (`--global`) starts whatever this says; turning it off here stops it for this
//!   session only.
//! - Windows: a value under `HKCU\…\CurrentVersion\Run`, through `reg.exe`.

use crate::settings::WhileQuit;
use std::sync::Arc;

#[cfg(target_os = "macos")]
pub mod launch_agent;

/// The launch agent's label on macOS, and its property list's name (with `.plist`).
pub const LABEL: &str = "io.github.PoHsuanLai.mailo.watch";

/// Where a login item stands once it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// It starts at each login, and was started now where the system does that.
    Done,
    /// macOS keeps it, and it waits for the person to allow it in Login Items.
    NeedsApproval,
}

/// Where the system keeps a program to start at login. Errors are the system's own words, for
/// the Settings sheet to show.
pub trait LoginItem: Send + Sync {
    /// Start `mailo watch` at each login from now on. Asking again when it already is changes
    /// nothing but where the program is.
    fn register(&self) -> Result<Registered, String>;
    /// Start it at login no more. Asking when it is not is done.
    fn unregister(&self) -> Result<(), String>;
}

/// The login item, as a root context. Only the launched app has one; a window without one
/// changes the switch in `settings.toml` and nothing else.
#[derive(Clone)]
pub struct LoginItems(pub Arc<dyn LoginItem>);

impl std::fmt::Debug for LoginItems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoginItems(..)")
    }
}

/// Make the login item what `wanted` says: registered when on, with where it stands; gone when
/// off (`None`).
pub fn apply(item: &dyn LoginItem, wanted: WhileQuit) -> Result<Option<Registered>, String> {
    match wanted {
        WhileQuit::On => item.register().map(Some),
        WhileQuit::Off => item.unregister().map(|()| None),
    }
}

/// This platform's login items, or `None` where mailo cannot name the program to start.
pub fn platform() -> Option<LoginItems> {
    #[cfg(target_os = "macos")]
    {
        let program = std::env::current_exe().ok()?;
        Some(LoginItems(Arc::new(launch_agent::Launchd { program })))
    }
    #[cfg(windows)]
    {
        let program = std::env::current_exe().ok()?;
        Some(LoginItems(Arc::new(RunKey { program })))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Some(LoginItems(Arc::new(UserUnit)))
    }
}

/// Run `program` with `args`, and say what it said on failure.
fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if said.is_empty() {
            format!("{program} {}: {}", args.join(" "), output.status)
        } else {
            said
        })
    }
}

/// The systemd user unit the packages install, as `systemctl --user` enables it.
#[cfg(not(any(target_os = "macos", windows)))]
struct UserUnit;

/// The unit's name, as `dist/install-lib.sh` installs it.
pub const UNIT: &str = "mailo-watch.service";

/// What `systemctl` is asked: `--user enable --now` the unit, or `disable --now` it.
pub fn systemctl_args(wanted: WhileQuit) -> [&'static str; 4] {
    match wanted {
        WhileQuit::On => ["--user", "enable", "--now", UNIT],
        WhileQuit::Off => ["--user", "disable", "--now", UNIT],
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
impl LoginItem for UserUnit {
    fn register(&self) -> Result<Registered, String> {
        run("systemctl", &systemctl_args(WhileQuit::On)).map(|_| Registered::Done)
    }

    fn unregister(&self) -> Result<(), String> {
        run("systemctl", &systemctl_args(WhileQuit::Off)).map(|_| ())
    }
}

/// The key a program started at this user's login is named under.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The value's name under [`RUN_KEY`].
pub const RUN_VALUE: &str = "mailo watch";

/// The command line the Run key holds: the program, quoted, and `watch`.
pub fn run_command(program: &std::path::Path) -> String {
    format!("\"{}\" watch", program.display())
}

/// The Run key, written with `reg.exe`.
#[cfg(windows)]
struct RunKey {
    program: std::path::PathBuf,
}

#[cfg(windows)]
impl LoginItem for RunKey {
    fn register(&self) -> Result<Registered, String> {
        let command = run_command(&self.program);
        run(
            "reg",
            &[
                "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
            ],
        )
        .map(|_| Registered::Done)
    }

    fn unregister(&self) -> Result<(), String> {
        // No value is nothing to delete, and `reg delete` would call it a failure.
        if run("reg", &["query", RUN_KEY, "/v", RUN_VALUE]).is_err() {
            return Ok(());
        }
        run("reg", &["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]).map(|_| ())
    }
}

#[cfg(test)]
mod tests;
