//! macOS's login item: the launch agent that runs `mailo watch`, kept by `SMAppService` where it
//! can be (macOS 13 on, mailo run from its bundle), else written to `~/Library/LaunchAgents` and
//! loaded with `launchctl`, as a launch agent was kept before `SMAppService`.

use super::{LABEL, LoginItem, Registered, run};
use mail_appkit::login::Unavailable;
use std::path::{Path, PathBuf};

/// The agent's property list in mailo.app's bundle, as `SMAppService` names it.
fn bundled() -> String {
    format!("{LABEL}.plist")
}

/// The launch agent, either way.
pub(super) struct Launchd {
    /// The program the agent written by hand runs: this one, where it is now.
    pub(super) program: PathBuf,
}

impl LoginItem for Launchd {
    fn register(&self) -> Result<Registered, String> {
        match mail_appkit::login::register_agent(&bundled()) {
            Ok(mail_appkit::login::Registered::Enabled) => Ok(Registered::Done),
            Ok(mail_appkit::login::Registered::NeedsApproval) => Ok(Registered::NeedsApproval),
            Err(Unavailable::NoService | Unavailable::NotInBundle) => {
                written(&self.program).map(|()| Registered::Done)
            }
            Err(Unavailable::Refused(why)) => Err(why),
        }
    }

    /// Both ways, so that whichever kept it, it goes.
    fn unregister(&self) -> Result<(), String> {
        let service = match mail_appkit::login::unregister_agent(&bundled()) {
            Ok(()) | Err(Unavailable::NoService | Unavailable::NotInBundle) => Ok(()),
            Err(Unavailable::Refused(why)) => Err(why),
        };
        unwritten()?;
        service
    }
}

/// The agent's property list for `program`: `program watch` at each login, started again after
/// it ends in failure (as the systemd unit is) and not after it ends of itself, which is a watch
/// that found another already running.
pub fn plist(program: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>watch</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ThrottleInterval</key>
	<integer>10</integer>
	<key>Nice</key>
	<integer>5</integer>
</dict>
</plist>
"#,
        escaped(&program.display().to_string())
    )
}

/// `text` as a property list's string holds it.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Where the agent written by hand goes: `~/Library/LaunchAgents/<label>.plist`.
fn path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("no home directory to keep a launch agent in")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// This user's launchd domain, `gui/<uid>`.
fn domain() -> Result<String, String> {
    let said = run("id", &["-u"])?;
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&said.stdout).trim()
    ))
}

/// Write the agent for `program` and load it: an agent already loaded with the same file is left
/// running, and one whose file changed (mailo moved) is loaded again.
fn written(program: &Path) -> Result<(), String> {
    let path = path()?;
    let text = plist(program);
    let domain = domain()?;
    let service = format!("{domain}/{LABEL}");
    let same = std::fs::read_to_string(&path).is_ok_and(|was| was == text);
    let loaded = run("launchctl", &["print", &service]).is_ok();
    if same && loaded {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    if loaded {
        // Loaded from the old file; the new one is bootstrapped next.
        let _ = run("launchctl", &["bootout", &service]);
    }
    let file = path.display().to_string();
    run("launchctl", &["bootstrap", &domain, &file]).map(|_| ())
}

/// Unload the agent written by hand, if it is loaded, and remove its file, if it is there.
fn unwritten() -> Result<(), String> {
    let path = path()?;
    let service = format!("{}/{LABEL}", domain()?);
    if run("launchctl", &["print", &service]).is_ok() {
        run("launchctl", &["bootout", &service])?;
    }
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}
