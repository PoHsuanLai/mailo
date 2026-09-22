//! A sync against a real IMAP4rev1 server implementation, not one written here.
//!
//! `imap_end_to_end.rs` speaks to a fake I wrote, so it agrees with my reading of RFC 3501 by
//! construction — and that reading is the thing that can be wrong. Twisted's `IMAP4Server` was
//! written by people who were not thinking about this client, and it renders literals, envelopes
//! and `FETCH` responses its own way.
//!
//! **`#[ignore]` and self-skipping**, for the same reason as `live_smtp.rs`. Start one:
//!
//! ```text
//! python3 -m venv venv && ./venv/bin/pip install twisted
//! ./venv/bin/python scripts/live-imapd.py 11143
//! cargo test -p mail-runtime --test live_imap -- --ignored --nocapture
//! ```

use mail_domain::{Credential, SaslMech, Tls};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{Transport, drive};
use tokio::sync::watch;

const PORT: u16 = 11143;

fn auth() -> ImapAuth {
    ImapAuth {
        username: "ada@example.test".to_owned(),
        credential: Credential::Password("s3cr3t-pass".to_owned()),
        sasl: vec![SaslMech::Plain],
    }
}

async fn run(commands: Vec<ImapCommand>) -> Option<mail_proto::ImapTranscript> {
    let mut transport = match Transport::connect("127.0.0.1", PORT, Tls::Plaintext).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: no IMAP server on 127.0.0.1:{PORT} ({e})");
            return None;
        }
    };
    let mut session = ImapSession::new(auth(), commands).expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);
    Some(
        drive(&mut session, &mut transport, &mut cancel)
            .await
            .expect("a real IMAP server accepts this client's bytes"),
    )
}

#[tokio::test]
#[ignore = "needs a local Twisted IMAP4 server; run deliberately with --ignored"]
async fn a_real_server_accepts_login_select_and_fetch() {
    let Some(transcript) = run(vec![
        ImapCommand::Capability,
        ImapCommand::Login,
        ImapCommand::Select {
            mailbox: "INBOX".to_owned(),
            read_only: true,
        },
        ImapCommand::UidFetch {
            set: "1:*".to_owned(),
            items: "(UID FLAGS RFC822.SIZE)".to_owned(),
        },
    ])
    .await
    else {
        return;
    };

    for line in &transcript.untagged {
        eprintln!("<- {}", line.text);
    }

    // The mailbox state the runtime needs to resume: without these a sync resurveys for ever.
    let all: String = transcript
        .untagged
        .iter()
        .map(|u| u.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all.contains("UIDVALIDITY"), "no UIDVALIDITY: {all}");
    assert!(
        all.contains("101") && all.contains("102"),
        "both UIDs should be reported: {all}"
    );
}

#[tokio::test]
#[ignore = "needs a local Twisted IMAP4 server; run deliberately with --ignored"]
async fn a_literal_body_survives_a_server_that_frames_it_its_own_way() {
    // The message contains `A1 OK not really` and a stray `)`. A parser that scans for a tagged
    // response or counts parentheses instead of honouring the literal's byte count truncates
    // there — and the fake server in imap_end_to_end.rs frames literals the way I framed them.
    let Some(transcript) = run(vec![
        ImapCommand::Login,
        ImapCommand::Select {
            mailbox: "INBOX".to_owned(),
            read_only: true,
        },
        ImapCommand::UidFetch {
            set: "102".to_owned(),
            items: "(UID BODY.PEEK[])".to_owned(),
        },
    ])
    .await
    else {
        return;
    };

    let all: String = transcript
        .untagged
        .iter()
        .map(|u| u.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!("fetched:\n{all}");
    assert!(
        all.contains("A1 OK not really"),
        "the body was truncated at the tag-shaped line: {all}"
    );
    assert!(
        all.contains("and a closing paren )"),
        "the body was truncated at the stray paren: {all}"
    );
}
