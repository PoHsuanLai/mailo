//! The lifecycle the app chooses and what it makes of the watch's lock. Nothing here starts a
//! thread, takes the person's lock or reads their settings.

use super::*;

#[test]
fn macos_stays_open_with_no_window_and_never_ends_by_itself() {
    assert!(stays_open(Os::MacOs));
}

#[test]
fn linux_and_windows_end_with_their_last_window_as_before() {
    assert!(!stays_open(Os::Other));
}

#[test]
fn this_build_stays_open_only_on_macos() {
    assert_eq!(stays_open(Os::HERE), cfg!(target_os = "macos"));
}

#[test]
fn the_app_watches_when_it_has_the_lock_and_waits_when_another_watch_has_it() {
    assert!(matches!(step(Ok::<_, Refused>(7)), Step::Watch(7)));
    // A `mailo watch` started by hand, or at login, keeps watching: the app never runs a second.
    assert!(matches!(
        step::<u32>(Err(Refused::AlreadyWatching)),
        Step::Wait
    ));
    // A lock that cannot be asked is no reason to watch beside whoever may hold it.
    assert!(matches!(
        step::<u32>(Err(Refused::Failed("no runtime directory".to_owned()))),
        Step::Wait
    ));
}

#[test]
fn the_app_s_lock_is_the_watch_s_own_and_refuses_a_second_holder() {
    // A lock in a scratch directory, never the person's: the app's claim and a `mailo watch`'s
    // are the same claim, so whichever comes second waits.
    let scratch = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let unique = scratch
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("background")
        .to_owned();
    let environment = latchkey::Environment {
        runtime_dir: Some(scratch.path().as_os_str().to_owned()),
        tmpdir: Some(scratch.path().as_os_str().to_owned()),
        home: None,
        local_app_data: Some(scratch.path().as_os_str().to_owned()),
        user: Some(unique),
    };
    let agent = latchkey::Agent::in_environment("mailo-bg-test", latchkey::here(), &environment)
        .unwrap_or_else(|e| panic!("{e}"));
    let first = step(mail_core::ipc::watching::claim_at(&agent));
    assert!(matches!(first, Step::Watch(_)), "{first:?}");
    let second = step(mail_core::ipc::watching::claim_at(&agent));
    assert!(matches!(second, Step::Wait), "{second:?}");
}
