//! What the daemon and its clients say to each other.
//!
//! Only the conversation is here. Where the door is, who may be behind it and how a client
//! starts one moved to `latchkey` along with their tests, because none of it was about mail —
//! and keeping a second copy here would mean two places to fix the next race found in it.

use mail_agent::ipc::wire;
use wire::{Mismatch, Request, Response};

#[test]
fn every_request_and_response_survives_the_round_trip() {
    for asked in [Request::Ping, Request::SyncNow, Request::Shutdown] {
        let text = wire::line(asked.clone()).unwrap();
        assert!(
            text.ends_with('\n'),
            "{asked:?}: the framing is the newline"
        );
        assert_eq!(wire::parse::<Request>(&text).unwrap(), asked, "{asked:?}");
    }
    for said in [Response::Started, Response::Stopping] {
        let text = wire::line(said.clone()).unwrap();
        assert!(text.ends_with('\n'), "{said:?}: the framing is the newline");
        assert_eq!(wire::parse::<Response>(&text).unwrap(), said, "{said:?}");
    }
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
        runtime_dir: Some(dir.as_os_str().to_owned()),
        tmpdir: Some(dir.as_os_str().to_owned()),
        home: None,
        local_app_data: Some(dir.as_os_str().to_owned()),
        user: Some(unique.to_owned()),
    };
    latchkey::Agent::in_environment(name, latchkey::here(), &environment).unwrap()
}

/// Say `line` at `agent`'s door as a client of any build would, and read the one answer.
fn knock(agent: &latchkey::Agent, line: &str) -> Response {
    use std::io::{BufRead, BufReader, Write};
    let mut stream = agent.connect().unwrap().unwrap();
    stream.write_all(line.as_bytes()).unwrap();
    let mut reply = String::new();
    BufReader::new(&mut stream).read_line(&mut reply).unwrap();
    wire::parse::<Response>(&reply).unwrap()
}

/// A watch's door holds nothing after it answers: a window from before 2026-10, which asked to
/// be told of each pass on a connection kept open, is refused and let go, and the door goes on
/// answering everyone else. Held connections were what ran a watch out of file descriptors.
#[test]
fn a_watch_answers_a_subscription_from_an_older_window_and_keeps_nothing() {
    use mail_agent::ipc::{client, watching};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-old-window");
    let _watching = watching::claim_at(&agent).unwrap();
    assert!(
        matches!(
            watching::claim_at(&agent),
            Err(watching::Refused::AlreadyWatching)
        ),
        "one watch per door"
    );

    let subscribe = format!("{{\"version\":{},\"body\":\"subscribe\"}}\n", wire::VERSION);
    for _ in 0..3 {
        let said = knock(&agent, &subscribe);
        assert!(matches!(said, Response::Refused(_)), "{said:?}");
    }
    // A client of another version is told which way round, and let go the same.
    let theirs = wire::VERSION + 1;
    assert_eq!(
        knock(
            &agent,
            &format!("{{\"version\":{theirs},\"body\":\"subscribe\"}}\n")
        ),
        Response::WrongVersion {
            daemon: wire::VERSION,
            client: theirs
        }
    );
    let pong = client::connect_at(&agent)
        .unwrap()
        .unwrap()
        .ask(Request::Ping)
        .unwrap();
    assert!(matches!(pong, Response::Pong { .. }), "{pong:?}");
}

/// A pass asked for at the watch's door is started, on the watch's scheduler, rather than
/// refused: the watch is the process that is fetching, so it is the one a Sync must reach.
/// Stopping is still refused there.
#[test]
fn a_watch_starts_a_pass_when_asked_and_is_not_stopped_through_its_door() {
    use mail_agent::ipc::{client, watching};
    let dir = tempfile::tempdir().unwrap();
    let agent = scratch(dir.path(), "mailo-watch-test-asked");
    let watching = watching::claim_at(&agent).unwrap();
    let asked = watching.asked();
    assert!(!asked.has_changed().unwrap(), "asked before anyone asked");

    let said = client::connect_at(&agent)
        .unwrap()
        .unwrap()
        .ask(Request::SyncNow)
        .unwrap();
    assert_eq!(said, Response::Started);
    // Moved before the answer went back, so it is already there to be read.
    assert!(
        asked.has_changed().unwrap(),
        "the ask never reached the watch's scheduler"
    );

    let said = client::connect_at(&agent)
        .unwrap()
        .unwrap()
        .ask(Request::Shutdown)
        .unwrap();
    assert!(matches!(said, Response::Refused(_)), "{said:?}");
}
