//! POP3 session transcripts. The replay harness drives [`Pop3Session`]; this file
//! does not contain a pump loop.

mod common;

use common::replay;
use mail_proto::{ListEntry, Pop3Command, Pop3Reply, Pop3Session, ProtoError, UidlEntry};

const USER: &str = "student";
const PASS: &str = "s3cret";

fn drive(commands: Vec<Pop3Command>, trace: &str) -> Vec<Pop3Reply> {
    let mut session = Pop3Session::new(USER, PASS, commands).expect("fixture credentials");
    replay(&mut session, trace).unwrap()
}

fn drive_err(commands: Vec<Pop3Command>, trace: &str) -> ProtoError {
    let mut session = Pop3Session::new(USER, PASS, commands).expect("fixture credentials");
    replay(&mut session, trace).unwrap_err()
}

fn message(lines: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[test]
fn greeting_capa_user_pass_and_quit() {
    let replies = drive(
        vec![
            Pop3Command::Capa,
            Pop3Command::User,
            Pop3Command::Pass,
            Pop3Command::Quit,
        ],
        include_str!("traces/pop3/auth_quit.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Capabilities(vec![
                "USER".into(),
                "SASL PLAIN LOGIN".into(),
                "UIDL".into(),
            ]),
            Pop3Reply::UserAccepted("student accepted".into()),
            Pop3Reply::Authenticated("maildrop ready".into()),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn capa_answered_with_err_is_not_a_failure() {
    let replies = drive(
        vec![
            Pop3Command::Capa,
            Pop3Command::User,
            Pop3Command::Pass,
            Pop3Command::Quit,
        ],
        include_str!("traces/pop3/capa_unsupported.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::CapaUnsupported("unrecognized command".into()),
            Pop3Reply::UserAccepted(String::new()),
            Pop3Reply::Authenticated(String::new()),
            Pop3Reply::Quit("signing off".into()),
        ]
    );
}

#[test]
fn uidl_lists_every_message() {
    let replies = drive(
        vec![
            Pop3Command::User,
            Pop3Command::Pass,
            Pop3Command::Uidl,
            Pop3Command::Quit,
        ],
        include_str!("traces/pop3/uidl.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::UserAccepted(String::new()),
            Pop3Reply::Authenticated("3 messages".into()),
            Pop3Reply::Uidl(vec![
                UidlEntry {
                    number: 1,
                    uidl: "0000000a4b2c1d3e".into(),
                },
                UidlEntry {
                    number: 2,
                    uidl: "0000000a4b2c1d3f".into(),
                },
                UidlEntry {
                    number: 3,
                    uidl: "abcd/ef+12".into(),
                },
            ]),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn an_empty_uidl_is_an_empty_list() {
    let replies = drive(
        vec![Pop3Command::Uidl, Pop3Command::Quit],
        include_str!("traces/pop3/uidl_empty.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Uidl(vec![]),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn retr_strips_dot_stuffing_and_keeps_message_crlf() {
    let replies = drive(
        vec![Pop3Command::Retr(1), Pop3Command::Quit],
        include_str!("traces/pop3/retr_dot_stuffed.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Retrieved(message(&[
                "From: ada@example.test",
                "Subject: dots",
                "",
                "Hello",
                ".a line that starts with a dot",
                ".",
                "Bye",
            ])),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn retr_reassembles_a_response_that_arrives_one_byte_at_a_time() {
    let replies = drive(
        vec![Pop3Command::Retr(2), Pop3Command::Quit],
        include_str!("traces/pop3/retr_split.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Retrieved(message(&["Hello", ".dot"])),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn retr_accepts_a_whole_multiline_reply_in_one_read() {
    let replies = drive(
        vec![Pop3Command::Retr(1), Pop3Command::Quit],
        include_str!("traces/pop3/retr_one_read.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Retrieved(message(&["Hello", ".dot"])),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn stat_list_and_dele() {
    let replies = drive(
        vec![
            Pop3Command::Stat,
            Pop3Command::List,
            Pop3Command::Dele(1),
            Pop3Command::Quit,
        ],
        include_str!("traces/pop3/transaction.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Stat {
                messages: 2,
                octets: 320,
            },
            Pop3Reply::List(vec![
                ListEntry {
                    number: 1,
                    octets: 120,
                },
                ListEntry {
                    number: 2,
                    octets: 200,
                },
            ]),
            Pop3Reply::Deleted("message 1 deleted".into()),
            Pop3Reply::Quit("dewey POP3 server signing off".into()),
        ]
    );
}

#[test]
fn auth_plain_sends_the_initial_response() {
    let replies = drive(
        vec![Pop3Command::AuthPlain, Pop3Command::Quit],
        include_str!("traces/pop3/auth_plain.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Authenticated("authenticated".into()),
            Pop3Reply::Quit("bye".into()),
        ]
    );
}

#[test]
fn a_rejected_password_is_auth_rejected_and_is_not_echoed() {
    let err = drive_err(
        vec![Pop3Command::User, Pop3Command::Pass],
        include_str!("traces/pop3/auth_rejected.trace"),
    );
    assert_eq!(
        err.to_string(),
        "authentication rejected: authentication failed"
    );
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(PASS), "auth error included the password");
}

#[test]
fn end_of_stream_mid_retr_is_unexpected_eof() {
    let err = drive_err(
        vec![Pop3Command::Retr(1)],
        include_str!("traces/pop3/unexpected_eof.trace"),
    );
    assert!(matches!(err, ProtoError::UnexpectedEof));
}

#[test]
fn end_of_stream_before_the_greeting_is_unexpected_eof() {
    let err = drive_err(
        vec![],
        include_str!("traces/pop3/eof_before_greeting.trace"),
    );
    assert!(matches!(err, ProtoError::UnexpectedEof));
}

#[test]
fn a_line_that_is_neither_ok_nor_err_is_malformed() {
    let err = drive_err(
        vec![Pop3Command::Stat],
        include_str!("traces/pop3/malformed.trace"),
    );
    match err {
        ProtoError::Malformed(text) => {
            assert!(text.contains("NOPE"), "malformed text was {text}");
        }
        other => panic!("expected Malformed, got {other}"),
    }
}

#[test]
fn retr_err_is_a_refusal_and_does_not_wait_for_a_dot() {
    let err = drive_err(
        vec![Pop3Command::Retr(9), Pop3Command::Quit],
        include_str!("traces/pop3/command_refused.trace"),
    );
    assert_eq!(err.to_string(), "server refused: no such message");
}

#[test]
fn interrupt_finishes_the_inflight_command_then_quits() {
    let replies = drive(
        vec![Pop3Command::Stat, Pop3Command::Uidl, Pop3Command::Quit],
        include_str!("traces/pop3/interrupt_skips_queue.trace"),
    );
    assert_eq!(
        replies,
        vec![
            Pop3Reply::Greeting("POP3 server ready".into()),
            Pop3Reply::Stat {
                messages: 2,
                octets: 320,
            },
            Pop3Reply::Quit("bye".into()),
        ]
    );
}
