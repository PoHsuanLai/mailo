//! What the daemon and its clients say to each other.
//!
//! Only the conversation is here. Where the door is, who may be behind it and how a client
//! starts one moved to `latchkey` along with their tests, because none of it was about mail —
//! and keeping a second copy here would mean two places to fix the next race found in it.

#[path = "../src/ipc/wire.rs"]
mod wire;

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
    assert!(said.contains("restart"), "{said}");
}

#[test]
fn rubbish_is_an_error_rather_than_a_panic() {
    assert!(matches!(
        wire::parse::<Request>("not json at all"),
        Err(Mismatch::Unreadable(_))
    ));
}
