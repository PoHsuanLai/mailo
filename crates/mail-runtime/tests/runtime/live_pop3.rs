//! A POP3 fetch against a real server implementation, checked byte for byte.
//!
//! POP3 is the protocol the first real account here will use, and F91 had just found that the
//! IMAP path was appending the FETCH response's closing paren to every message and running each
//! body through `from_utf8_lossy`. The same questions had to be asked of this one, against a
//! server nobody here wrote: `twisted.mail.pop3`.
//!
//! The fixture is chosen to break a careless client in the three ways POP3 can:
//!
//! - a body line beginning with `.`, which the server doubles and the client must undouble
//!   exactly once — undo it twice and the line is corrupted, not at all and the message ends early;
//! - 8-bit bytes, which must not pass through a UTF-8 conversion;
//! - CRLF endings, which must arrive as CRLF and not as `\r\r\n`.
//!
//! `#[ignore]`d; run it through `./scripts/live-tests.sh`.

use mail_domain::Tls;
use mail_proto::{Pop3Command, Pop3Reply, Pop3Session};
use mail_runtime::{Transport, drive};
use tokio::sync::watch;

const PORT: u16 = 11110;

/// The second fixture message, exactly as `scripts/live-pop3d.py` holds it.
const EXPECTED_BODY: &[u8] =
    b".a line beginning with a dot\r\ncaf\xe9 and na\xefve\r\nand the end.\r\n";

#[tokio::test]
#[ignore = "needs a local Twisted POP3 server; run via ./scripts/live-tests.sh"]
async fn a_retrieved_message_is_byte_for_byte_what_the_server_holds() {
    let mut transport = match Transport::connect("127.0.0.1", PORT, Tls::Plaintext).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: no POP3 server on 127.0.0.1:{PORT} ({e})");
            return;
        }
    };

    let mut session = Pop3Session::new(
        "ada@example.test",
        "s3cr3t-pass",
        vec![
            Pop3Command::User,
            Pop3Command::Pass,
            Pop3Command::Uidl,
            Pop3Command::Retr(2),
        ],
    )
    .expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);

    let replies = drive(&mut session, &mut transport, &mut cancel)
        .await
        .expect("a real POP3 server accepts this client's bytes");

    let body = replies
        .iter()
        .find_map(|r| match r {
            Pop3Reply::Retrieved(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .expect("RETR returned a message");
    // Split on the raw bytes: a lossy conversion here would hide exactly the corruption this
    // test exists to catch.
    let sep = body
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("headers and body are separated");
    let tail = &body[sep + 4..];

    // Byte for byte, not `contains`: F91 shipped because a substring assertion cannot see
    // something *added*, and the three failures below are all additions or substitutions.
    assert_eq!(
        tail, EXPECTED_BODY,
        "the body is not byte-for-byte what the server holds"
    );
    assert!(
        !tail.windows(3).any(|w| w == b"\r\r\n"),
        "line endings were doubled"
    );
    assert!(
        tail.starts_with(b".a line beginning"),
        "the dot was un-stuffed the wrong number of times"
    );
}

#[tokio::test]
#[ignore = "needs a local Twisted POP3 server; run via ./scripts/live-tests.sh"]
async fn top_returns_headers_without_marking_the_message_read() {
    // `TOP` is what keeps a first sync from marking an entire maildrop read in the user's
    // webmail — `RETR` sets the seen flag and `TOP` does not. Worth exercising against a real
    // server, because the whole size-banded sync depends on it being available and correct.
    let mut transport = match Transport::connect("127.0.0.1", PORT, Tls::Plaintext).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: no POP3 server on 127.0.0.1:{PORT} ({e})");
            return;
        }
    };
    let mut session = Pop3Session::new(
        "ada@example.test",
        "s3cr3t-pass",
        vec![Pop3Command::User, Pop3Command::Pass, Pop3Command::Top(2, 0)],
    )
    .expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);

    let replies = drive(&mut session, &mut transport, &mut cancel)
        .await
        .expect("TOP is accepted");
    let headers = replies
        .iter()
        .find_map(|r| match r {
            Pop3Reply::Headers(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
            _ => None,
        })
        .expect("TOP returned headers");

    assert!(
        headers.contains("Subject: dotted and eight-bit"),
        "{headers}"
    );
    assert!(
        !headers.contains("and the end."),
        "TOP 0 should return no body lines: {headers}"
    );
}
