//! Submission against a real SMTP server implementation, not one written here.
//!
//! The fake server in `submission_end_to_end.rs` is mine, so it agrees with my reading of the
//! protocol by construction — which is the thing that can be wrong. `aiosmtpd` is a mail server
//! written by people who were not thinking about this client, and it checks the parts a fake
//! cannot: that `AUTH PLAIN`'s base64 decodes to credentials a server recognises, that the
//! envelope commands are accepted as written, and that dot-stuffed `DATA` arrives as the bytes
//! that went in.
//!
//! **`#[ignore]` and self-skipping.** It needs a server this repository does not ship. Start one:
//!
//! ```text
//! python3 -m venv venv && ./venv/bin/pip install aiosmtpd
//! ./venv/bin/python smtpd.py 12525 /tmp/received.eml
//! cargo test -p mail-runtime --test live_smtp -- --ignored --nocapture
//! ```
//!
//! If nothing is listening the test says so and returns, because a test that fails when a
//! developer has not started a daemon is a test people learn to ignore.

use mail_domain::{Credential, SaslMech, Tls};
use mail_proto::{SmtpSession, Submission};
use mail_runtime::{Transport, drive};
use tokio::sync::watch;

const PORT: u16 = 12525;
const USER: &str = "ada@example.test";
const PASS: &str = "s3cr3t-pass";

/// A body chosen to break a careless client: a line that is a bare dot after stuffing, a line
/// that looks like a terminator, and non-ASCII.
const BODY: &str = "From: Ada <ada@example.test>\r\n\
To: Bob <bob@example.test>\r\n\
Subject: =?utf-8?B?ZMOpasOgIHZ1?=\r\n\
\r\n\
ordinary line\r\n\
.\r\n\
.hidden leading dot\r\n\
..already doubled\r\n\
last line\r\n";

#[tokio::test]
#[ignore = "needs a local aiosmtpd; run deliberately with --ignored"]
async fn a_real_server_accepts_what_this_client_sends() {
    let mut transport = match Transport::connect("127.0.0.1", PORT, Tls::Plaintext).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: no SMTP server on 127.0.0.1:{PORT} ({e})");
            return;
        }
    };

    let submission = Submission {
        ehlo: "mailo.test".to_owned(),
        host: "127.0.0.1".to_owned(),
        port: PORT,
        tls: Tls::Plaintext,
        username: USER.to_owned(),
        credential: Credential::Password(PASS.to_owned()),
        sasl: vec![SaslMech::Plain],
        mail_from: USER.to_owned(),
        // Two recipients, so a client that sends one RCPT TO for a list is caught.
        recipients: vec![
            "bob@example.test".to_owned(),
            "cara@example.test".to_owned(),
        ],
        message: BODY.as_bytes().to_vec(),
    };

    let mut session = SmtpSession::new(submission);
    let (_tx, mut cancel) = watch::channel(false);

    let reply = drive(&mut session, &mut transport, &mut cancel)
        .await
        .expect("a real SMTP server accepts this client's bytes");
    eprintln!("server accepted: {reply:?}");

    // What the server *received*, which is the only place dot-stuffing can be checked. The
    // script writes the envelope and then the un-stuffed body.
    let path = std::env::var("MAILO_RECEIVED").unwrap_or_else(|_| "/tmp/received.eml".to_owned());
    let Ok(received) = std::fs::read_to_string(&path) else {
        eprintln!("no {path}; skipping the byte-for-byte check");
        return;
    };
    let (envelope, body) = received
        .split_once("---\n")
        .expect("the script writes an envelope, then ---, then the body");

    assert!(
        envelope.contains("bob@example.test") && envelope.contains("cara@example.test"),
        "both recipients should be separate RCPT TO commands: {envelope}"
    );

    // The three lines a careless client corrupts. A bare `.` is the DATA terminator itself, so
    // a client that does not stuff it truncates the message there and the server never sees the
    // rest; one that over-stuffs leaves the extra dot in the delivered mail.
    assert!(
        body.contains("\n.\r\n") || body.starts_with(".\r\n"),
        "a line containing only a dot did not survive:\n{body:?}"
    );
    assert!(
        body.contains(".hidden leading dot"),
        "a line beginning with a dot was altered:\n{body:?}"
    );
    assert!(
        body.contains("..already doubled"),
        "an already-doubled dot was changed:\n{body:?}"
    );
    assert!(
        body.contains("last line"),
        "the message was truncated at the dotted line:\n{body:?}"
    );
    // Checked by disabling `stuff_line` and running this: the bare `.` then ends DATA early,
    // the server reads the rest of the message as SMTP commands, and the exchange deadlocks
    // waiting for a reply that will never take the expected shape. The corruption is not subtle
    // once a real server is on the other end — it is invisible against a fake that unstuffs
    // whatever it is given.
}

#[tokio::test]
#[ignore = "needs a local aiosmtpd; run deliberately with --ignored"]
async fn a_real_server_rejecting_a_password_is_an_auth_error_not_a_crash() {
    // The error path, against a server that decides for itself. A client that mishandles a 535
    // either retries a credential that will never work or reports the wrong cause — and this is
    // the one place it can be checked without spending someone else's rate limit.
    let mut transport = match Transport::connect("127.0.0.1", PORT, Tls::Plaintext).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: no SMTP server on 127.0.0.1:{PORT} ({e})");
            return;
        }
    };

    let submission = Submission {
        ehlo: "mailo.test".to_owned(),
        host: "127.0.0.1".to_owned(),
        port: PORT,
        tls: Tls::Plaintext,
        username: USER.to_owned(),
        credential: Credential::Password("definitely-not-the-password".to_owned()),
        sasl: vec![SaslMech::Plain],
        mail_from: USER.to_owned(),
        recipients: vec!["bob@example.test".to_owned()],
        message: BODY.as_bytes().to_vec(),
    };

    let mut session = SmtpSession::new(submission);
    let (_tx, mut cancel) = watch::channel(false);
    let outcome = drive(&mut session, &mut transport, &mut cancel).await;

    let err = outcome.expect_err("a wrong password must not look like a delivery");
    let text = format!("{err}");
    eprintln!("rejected as: {text}");
    // Reported as authentication, so the runtime asks for a credential rather than retrying
    // the same one until the backoff runs out.
    assert!(
        text.to_lowercase().contains("auth") || text.contains("535"),
        "a rejected password should name authentication: {text}"
    );
}
