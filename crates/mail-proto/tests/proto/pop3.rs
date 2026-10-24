//! POP3 session transcripts. The replay harness drives [`Pop3Session`]; this file
//! does not contain a pump loop.

use crate::common;

use common::replay;
use mail_proto::{ListEntry, Pop3Command, Pop3Reply, Pop3Session, ProtoError, Refusal, UidlEntry};

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

/// `(what the session shows, its commands, its transcript, the replies after the greeting)`,
/// each a session that ends well.
fn sessions() -> Vec<(&'static str, Vec<Pop3Command>, &'static str, Vec<Pop3Reply>)> {
    vec![
        (
            "greeting, CAPA, USER, PASS and QUIT",
            vec![
                Pop3Command::Capa,
                Pop3Command::User,
                Pop3Command::Pass,
                Pop3Command::Quit,
            ],
            include_str!("../traces/pop3/auth_quit.trace"),
            vec![
                Pop3Reply::Capabilities(vec![
                    "USER".into(),
                    "SASL PLAIN LOGIN".into(),
                    "UIDL".into(),
                ]),
                Pop3Reply::UserAccepted("student accepted".into()),
                Pop3Reply::Authenticated("maildrop ready".into()),
                Pop3Reply::Quit("bye".into()),
            ],
        ),
        (
            "CAPA answered with -ERR is not a failure",
            vec![
                Pop3Command::Capa,
                Pop3Command::User,
                Pop3Command::Pass,
                Pop3Command::Quit,
            ],
            include_str!("../traces/pop3/capa_unsupported.trace"),
            vec![
                Pop3Reply::CapaUnsupported("unrecognized command".into()),
                Pop3Reply::UserAccepted(String::new()),
                Pop3Reply::Authenticated(String::new()),
                Pop3Reply::Quit("signing off".into()),
            ],
        ),
        (
            "UIDL lists every message",
            vec![
                Pop3Command::User,
                Pop3Command::Pass,
                Pop3Command::Uidl,
                Pop3Command::Quit,
            ],
            include_str!("../traces/pop3/uidl.trace"),
            vec![
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
            ],
        ),
        (
            "an empty UIDL is an empty list",
            vec![Pop3Command::Uidl, Pop3Command::Quit],
            include_str!("../traces/pop3/uidl_empty.trace"),
            vec![Pop3Reply::Uidl(vec![]), Pop3Reply::Quit("bye".into())],
        ),
        (
            "RETR strips dot-stuffing and keeps the message's CRLF",
            vec![Pop3Command::Retr(1), Pop3Command::Quit],
            include_str!("../traces/pop3/retr_dot_stuffed.trace"),
            vec![
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
            ],
        ),
        (
            "RETR reassembles a reply that arrives one byte at a time",
            vec![Pop3Command::Retr(2), Pop3Command::Quit],
            include_str!("../traces/pop3/retr_split.trace"),
            vec![
                Pop3Reply::Retrieved(message(&["Hello", ".dot"])),
                Pop3Reply::Quit("bye".into()),
            ],
        ),
        (
            "RETR accepts a whole multiline reply in one read",
            vec![Pop3Command::Retr(1), Pop3Command::Quit],
            include_str!("../traces/pop3/retr_one_read.trace"),
            vec![
                Pop3Reply::Retrieved(message(&["Hello", ".dot"])),
                Pop3Reply::Quit("bye".into()),
            ],
        ),
        (
            "STAT, LIST and DELE",
            vec![
                Pop3Command::Stat,
                Pop3Command::List,
                Pop3Command::Dele(1),
                Pop3Command::Quit,
            ],
            include_str!("../traces/pop3/transaction.trace"),
            vec![
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
            ],
        ),
        (
            "AUTH PLAIN sends the initial response",
            vec![Pop3Command::AuthPlain, Pop3Command::Quit],
            include_str!("../traces/pop3/auth_plain.trace"),
            vec![
                Pop3Reply::Authenticated("authenticated".into()),
                Pop3Reply::Quit("bye".into()),
            ],
        ),
        (
            "an interrupt finishes the command in flight, then quits",
            vec![Pop3Command::Stat, Pop3Command::Uidl, Pop3Command::Quit],
            include_str!("../traces/pop3/interrupt_skips_queue.trace"),
            vec![
                Pop3Reply::Stat {
                    messages: 2,
                    octets: 320,
                },
                Pop3Reply::Quit("bye".into()),
            ],
        ),
        // `TOP n 0` returns headers and does not claim to be a whole message. `RETR` sets the
        // seen flag on Dovecot and `TOP` does not, so a first sync over a large maildrop must
        // use `TOP` or it marks the whole mailbox read in webmail; a reply of its own stops a
        // caller storing headers as a full message.
        (
            "TOP returns headers without claiming a whole message",
            vec![Pop3Command::Top(1, 2), Pop3Command::Quit],
            include_str!("../traces/pop3/top_headers_only.trace"),
            vec![
                Pop3Reply::Headers(message(&[
                    "From: ada@example.test",
                    "Subject: headers and two lines",
                    "",
                    "Hello there",
                    // sent as "..signature follows": one leading dot removed, exactly as in RETR
                    ".signature follows",
                ])),
                Pop3Reply::Quit("bye".into()),
            ],
        ),
    ]
}

#[test]
fn each_session_replies_as_its_transcript_says() {
    for (shows, commands, trace, after_greeting) in sessions() {
        let mut want = vec![Pop3Reply::Greeting("POP3 server ready".into())];
        want.extend(after_greeting);
        assert_eq!(drive(commands, trace), want, "{shows}");
    }
}

#[test]
fn a_rejected_password_is_auth_rejected_and_is_not_echoed() {
    let err = drive_err(
        vec![Pop3Command::User, Pop3Command::Pass],
        include_str!("../traces/pop3/auth_rejected.trace"),
    );
    assert_eq!(
        err.to_string(),
        "authentication rejected: authentication failed"
    );
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(PASS), "auth error included the password");
}

#[test]
fn end_of_stream_before_the_greeting_or_mid_retr_is_unexpected_eof() {
    for (commands, trace) in [
        (
            vec![],
            include_str!("../traces/pop3/eof_before_greeting.trace"),
        ),
        (
            vec![Pop3Command::Retr(1)],
            include_str!("../traces/pop3/unexpected_eof.trace"),
        ),
    ] {
        let err = drive_err(commands, trace);
        assert!(matches!(err, ProtoError::UnexpectedEof), "{err:?}");
    }
}

#[test]
fn a_line_that_is_neither_ok_nor_err_is_malformed() {
    let err = drive_err(
        vec![Pop3Command::Stat],
        include_str!("../traces/pop3/malformed.trace"),
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
        include_str!("../traces/pop3/command_refused.trace"),
    );
    // POP3 has no transient/permanent distinction on the wire, so -ERR outside authentication
    // is Permanent: the outbox undoes rather than retrying a command the server just rejected.
    assert!(
        matches!(
            &err,
            ProtoError::Refused {
                kind: Refusal::Permanent,
                text
            } if text == "no such message"
        ),
        "{err:?}"
    );
}
