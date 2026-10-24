//! The switch handed to the seam, and what each platform's keeper would be told. Nothing here
//! registers a login item, runs `launchctl` or `systemctl`, or writes the registry.

use super::*;
use std::sync::Mutex;

/// A login item that records what it was asked, and answers as told.
#[derive(Default)]
struct Recorder {
    asked: Mutex<Vec<&'static str>>,
    refuse: Option<String>,
}

impl LoginItem for Recorder {
    fn register(&self) -> Result<Registered, String> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("register");
        match &self.refuse {
            Some(why) => Err(why.clone()),
            None => Ok(Registered::Done),
        }
    }

    fn unregister(&self) -> Result<(), String> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push("unregister");
        self.refuse.clone().map_or(Ok(()), Err)
    }
}

#[test]
fn on_registers_the_login_item_and_off_unregisters_it() {
    let item = Recorder::default();
    assert_eq!(apply(&item, WhileQuit::On), Ok(Some(Registered::Done)));
    assert_eq!(apply(&item, WhileQuit::Off), Ok(None));
    assert_eq!(
        *item.asked.lock().unwrap_or_else(|e| e.into_inner()),
        ["register", "unregister"]
    );
}

#[test]
fn what_the_system_refused_comes_back_in_its_words() {
    let item = Recorder {
        refuse: Some("Operation not permitted".to_owned()),
        ..Recorder::default()
    };
    assert_eq!(
        apply(&item, WhileQuit::On),
        Err("Operation not permitted".to_owned())
    );
    assert_eq!(
        apply(&item, WhileQuit::Off),
        Err("Operation not permitted".to_owned())
    );
}

#[test]
fn it_is_off_until_the_person_turns_it_on() {
    assert_eq!(WhileQuit::default(), WhileQuit::Off);
}

#[test]
fn systemd_enables_and_disables_the_installed_unit_for_this_user_now() {
    assert_eq!(
        systemctl_args(WhileQuit::On),
        ["--user", "enable", "--now", "mailo-watch.service"]
    );
    assert_eq!(
        systemctl_args(WhileQuit::Off),
        ["--user", "disable", "--now", "mailo-watch.service"]
    );
}

#[test]
fn the_run_key_starts_the_quoted_program_watching() {
    assert_eq!(
        run_command(std::path::Path::new(r"C:\Program Files\mailo\mailo.exe")),
        r#""C:\Program Files\mailo\mailo.exe" watch"#
    );
    assert!(
        RUN_KEY.starts_with(r"HKCU\"),
        "this user's, not the machine's"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_launch_agent_runs_this_program_watching_and_restarts_it_only_after_a_failure() {
    let text = launch_agent::plist(std::path::Path::new(
        "/Applications/M&M <1>.app/Contents/MacOS/mailo",
    ));
    assert!(text.contains(&format!("<string>{LABEL}</string>")));
    assert!(text.contains(
        "<string>/Applications/M&amp;M &lt;1&gt;.app/Contents/MacOS/mailo</string>\n\t\t<string>watch</string>"
    ));
    assert!(text.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
    assert!(text.contains("<key>RunAtLoad</key>\n\t<true/>"));
}

#[cfg(target_os = "macos")]
#[test]
fn the_bundled_launch_agent_is_the_one_bundle_sh_puts_in_the_app() {
    let script = include_str!("../../../../packaging/macos/bundle.sh");
    assert!(script.contains(&format!("{LABEL}.plist")), "{LABEL}");
    let bundled = include_str!("../../../../packaging/macos/watch.plist");
    assert!(bundled.contains(&format!("<string>{LABEL}</string>")));
    assert!(bundled.contains("<key>BundleProgram</key>"));
}
