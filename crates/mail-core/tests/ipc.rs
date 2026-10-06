//! What the daemon and its clients say to each other.
//!
//! Only the conversation is here. Where the door is, who may be behind it and how a client
//! starts one moved to `latchkey` along with their tests, because none of it was about mail —
//! and keeping a second copy here would mean two places to fix the next race found in it.

use mail_core::ipc::wire;
use wire::{Mismatch, Request, Response};

#[test]
fn a_request_survives_the_round_trip() {
    let text = wire::line(Request::SyncNow).unwrap();
    assert!(text.ends_with('\n'), "the framing is the newline");
    assert_eq!(wire::parse::<Request>(&text).unwrap(), Request::SyncNow);
}

#[test]
fn a_message_never_contains_its_own_terminator() {
    // The framing is a newline, so a value carrying one would end the message early and the
    // rest would be read as the next one. JSON escapes them; this is the property the
    // framing depends on, asserted rather than assumed.
    let text = wire::line(Response::Refused("two\nlines".to_owned())).unwrap();
    assert_eq!(text.matches('\n').count(), 1, "{text:?}");
    assert_eq!(
        wire::parse::<Response>(&text).unwrap(),
        Response::Refused("two\nlines".to_owned())
    );
}

#[test]
fn a_version_this_build_does_not_speak_is_refused_before_the_body_is_read() {
    // A daemon left running across an upgrade is the normal case — `watch` holds IDLE
    // connections for hours and nobody restarts it to install a binary — so the first thing
    // a new client meets is an old daemon. Reading the body under the old meaning of a
    // changed field is the failure this prevents.
    let stale = r#"{"version":999,"body":"ping"}"#;
    match wire::parse::<Request>(stale) {
        Err(Mismatch::Version { theirs, ours }) => {
            assert_eq!(theirs, 999);
            assert_eq!(ours, wire::VERSION);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_mismatch_says_which_way_round_it_is() {
    // Which side is older decides the remedy — restart the daemon, or upgrade the client —
    // so both numbers have to reach the person reading it.
    let said = Mismatch::Version { theirs: 2, ours: 1 }.to_string();
    assert!(said.contains('2') && said.contains('1'), "{said}");
    assert!(said.contains("start it again"), "{said}");
}

#[test]
fn rubbish_is_an_error_rather_than_a_panic() {
    assert!(matches!(
        wire::parse::<Request>("not json at all"),
        Err(Mismatch::Unreadable(_))
    ));
}

#[test]
fn a_subscription_and_what_it_hears_survive_the_round_trip() {
    let text = wire::line(Request::Subscribe).unwrap();
    assert_eq!(wire::parse::<Request>(&text).unwrap(), Request::Subscribe);
    let account = mail_domain::id::new_account_id();
    for said in [Response::Subscribed, Response::Changed { account }] {
        let text = wire::line(said.clone()).unwrap();
        assert_eq!(wire::parse::<Response>(&text).unwrap(), said);
    }
}

/// A door in `dir`, never the person's, as this platform makes one: a socket in `dir`, or on
/// Windows a named pipe with its lock in `dir`.
fn scratch(dir: &std::path::Path, name: &str) -> latchkey::Agent {
    // The test directory's own name keeps two runs' pipes apart: a pipe's namespace is the
    // machine's, not the directory's.
    let unique = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("test");
    let environment = latchkey::Environment {
        runtime_dir: Some(dir.as_os_str()),
        tmpdir: Some(dir.as_os_str()),
        home: None,
        local_app_data: Some(dir.as_os_str()),
        user: Some(unique),
    };
    latchkey::Agent::in_environment(name, latchkey::here(), &environment).unwrap()
}

/// Wait, briefly, for `ready`; a door's threads answer in their own time.
fn until(ready: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    ready()
}

#[test]
fn a_watch_tells_whoever_subscribed_of_each_pass_in_order() {
    use mail_core::ipc::{client, watching};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-told");
    assert!(
        client::subscribe(&agent).unwrap().is_none(),
        "no door, so nothing to subscribe to"
    );
    let watching = watching::claim_at(&agent).unwrap();
    assert!(
        matches!(
            watching::claim_at(&agent),
            Err(watching::Refused::AlreadyWatching)
        ),
        "one watch per door"
    );

    let mut first = client::subscribe(&agent).unwrap().unwrap();
    let mut second = client::subscribe(&agent).unwrap().unwrap();
    // Subscribed, the door still answers: a listener that never hangs up does not keep it shut.
    let pong = client::connect_at(&agent)
        .unwrap()
        .unwrap()
        .ask(Request::Ping)
        .unwrap();
    assert!(matches!(pong, Response::Pong { .. }), "{pong:?}");
    assert_eq!(watching.listeners(), 2);

    let (a, b) = (
        mail_domain::id::new_account_id(),
        mail_domain::id::new_account_id(),
    );
    watching.changed(a.clone());
    watching.changed(b.clone());
    for changes in [&mut first, &mut second] {
        assert_eq!(changes.wait().unwrap(), a);
        assert_eq!(changes.wait().unwrap(), b);
    }

    // One leaves. It is found gone the next time it is written to, so tell until it is.
    drop(first);
    assert!(
        until(|| {
            watching.changed(a.clone());
            watching.listeners() == 1
        }),
        "a listener that left was kept"
    );
    // The one that stayed heard all of those, and goes on hearing.
    watching.changed(b.clone());
    assert!(
        std::iter::from_fn(|| second.wait().ok())
            .take(10_000)
            .any(|heard| heard == b),
        "the listener that stayed stopped hearing"
    );
}

#[test]
fn a_watch_refuses_the_daemons_work() {
    use mail_core::ipc::{client, watching};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-refuses");
    let _watching = watching::claim_at(&agent).unwrap();
    for request in [Request::SyncNow, Request::Shutdown] {
        let said = client::connect_at(&agent)
            .unwrap()
            .unwrap()
            .ask(request)
            .unwrap();
        assert!(matches!(said, Response::Refused(_)), "{said:?}");
    }
}

#[test]
fn a_subscription_from_another_build_is_told_which_way_round_it_is() {
    use mail_core::ipc::watching;
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-old-client");
    let watching = watching::claim_at(&agent).unwrap();
    // A client of another version, asking under its own.
    let theirs = wire::VERSION + 1;
    let mut stream = agent.connect().unwrap().unwrap();
    stream
        .write_all(format!("{{\"version\":{theirs},\"body\":\"subscribe\"}}\n").as_bytes())
        .unwrap();
    let mut reply = String::new();
    BufReader::new(&mut stream).read_line(&mut reply).unwrap();
    assert_eq!(
        wire::parse::<Response>(&reply).unwrap(),
        Response::WrongVersion {
            daemon: wire::VERSION,
            client: theirs
        }
    );
    assert_eq!(
        watching.listeners(),
        0,
        "subscribed under a version it did not speak"
    );
}

#[test]
fn subscribing_to_a_daemon_from_another_build_is_an_error_that_names_the_remedy() {
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-old-daemon");
    let listening = agent.listen().unwrap();
    let old = std::thread::spawn(move || {
        let mut stream = listening.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).unwrap();
        // What a daemon of another version says to a request under this one.
        let (theirs, ours) = (wire::VERSION + 1, wire::VERSION);
        let said = format!(
            "{{\"version\":{theirs},\"body\":{{\"wrong_version\":{{\"daemon\":{theirs},\"client\":{ours}}}}}}}\n"
        );
        stream.write_all(said.as_bytes()).unwrap();
        stream.flush().unwrap();
        listening
    });
    let why = mail_core::ipc::client::subscribe(&agent).unwrap_err();
    assert!(why.contains("start it again"), "{why}");
    drop(old.join().unwrap());
}
