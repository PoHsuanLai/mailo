//! The lifecycle the app chooses and what it makes of the watch's lock. Nothing here starts a
//! thread, takes the person's lock or reads their settings.

use super::*;
use ds_settings::ConfigRoot;

#[test]
fn macos_stays_open_with_no_window_and_never_ends_by_itself() {
    assert!(stays_open(Os::MacOs));
    assert_eq!(
        last_window(Os::MacOs),
        LastWindowClosed::StayFor(Duration::MAX)
    );
    // The linger is too long to add to any instant, which is what makes ds-blitz run for ever.
    assert!(
        std::time::Instant::now()
            .checked_add(Duration::MAX)
            .is_none()
    );
}

#[test]
fn linux_and_windows_end_with_their_last_window_as_before() {
    assert!(!stays_open(Os::Other));
    assert_eq!(last_window(Os::Other), LastWindowClosed::Exit);
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

#[test]
fn new_mail_is_said_as_the_settings_say_now() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = ConfigRoot::Scratch(dir.path().to_path_buf());
    assert!(says_new_mail(None), "on as it ships");
    assert!(says_new_mail(Some(&root)), "no file is the defaults");
    crate::settings::change(&root, |settings| {
        settings.notifications.new_mail = crate::settings::NewMail::Off;
    })
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(!says_new_mail(Some(&root)));
}
