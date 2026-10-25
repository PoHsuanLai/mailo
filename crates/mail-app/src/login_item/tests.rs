//! The switch handed to the seam, and what each platform's keeper would be told. Nothing here
//! registers a login item, runs `launchctl`, or writes the registry.

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

#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn linux_and_the_bsds_have_no_login_item_for_the_switch_to_keep() {
    // The unit `dist/install.sh` enables for every user checks mail whatever a switch says.
    assert!(!KEEPS);
    assert!(platform().is_none());
}

#[test]
fn the_run_key_starts_the_quoted_program_waiting_to_watch_with_no_console_window() {
    assert_eq!(
        run_command(std::path::Path::new(r"C:\Program Files\mailo\mailo.exe")),
        r#"conhost.exe --headless "C:\Program Files\mailo\mailo.exe" watch --wait"#
    );
    assert!(
        RUN_KEY.starts_with(r"HKCU\"),
        "this user's, not the machine's"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_launch_agent_runs_this_program_waiting_to_watch_and_restarts_it_only_after_a_failure() {
    let text = launch_agent::plist(std::path::Path::new(
        "/Applications/M&M <1>.app/Contents/MacOS/mailo",
    ));
    assert!(text.contains(&format!("<string>{LABEL}</string>")));
    assert!(text.contains(
        "<string>/Applications/M&amp;M &lt;1&gt;.app/Contents/MacOS/mailo</string>\n\t\t<string>watch</string>\n\t\t<string>--wait</string>"
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
    // A watch that finds mailo.app watching waits for ⌘Q rather than ending: launchd would not
    // start one that ended of itself again.
    assert!(bundled.contains("<string>watch</string>\n\t\t<string>--wait</string>"));
}
