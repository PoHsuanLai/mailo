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
//!   `launchctl` ([`launch_agent`]). The agent runs `mailo watch --wait`, so one started while
//!   mailo.app watches takes over once ⌘Q ends the app.
//! - Windows: a value under `HKCU\…\CurrentVersion\Run`, through `reg.exe`, run by a headless
//!   console host so no window opens at login.
//! - Linux and the BSDs: none. `dist/install.sh` enables the systemd user unit for every user, so
//!   mail is checked with no window open there whatever a switch would say; Settings shows none
//!   ([`KEEPS`]).

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

/// Make the login item what `wanted` says: registered when `wanted`, with where it stands; gone
/// when not (`None`). The Settings sheet's switch is `wanted`.
pub fn apply(item: &dyn LoginItem, wanted: bool) -> Result<Option<Registered>, String> {
    if wanted {
        item.register().map(Some)
    } else {
        item.unregister().map(|()| None)
    }
}

/// Whether this platform has a login item for the switch to keep: macOS and Windows.
pub const KEEPS: bool = cfg!(any(target_os = "macos", windows));

/// This platform's login items, or `None` where there are none ([`KEEPS`]) or mailo cannot name
/// the program to start.
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
        None
    }
}

/// Run `program` with `args`, and say what it said on failure. On Windows it opens no console
/// window of its own.
#[cfg(any(target_os = "macos", windows))]
fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
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

/// The process creation flag that keeps a console program from opening a window of its own.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The key a program started at this user's login is named under.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The value's name under [`RUN_KEY`].
pub const RUN_VALUE: &str = "mailo watch";

/// The command line the Run key holds: the program, quoted, and `watch --wait`, under the console
/// host with no window (`conhost.exe --headless`), since mailo is a console program and Windows
/// would otherwise open a console for it at each login.
pub fn run_command(program: &std::path::Path) -> String {
    format!(
        "conhost.exe --headless \"{}\" watch --wait",
        program.display()
    )
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
