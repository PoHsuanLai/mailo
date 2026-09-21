//! The one test here that talks to a server nobody in this repository wrote.
//!
//! Every other test drives a fake: transcripts, or a `TcpListener` in this process answering
//! what I decided it should answer. That validates the client against my own understanding of
//! the protocol, which is exactly the thing that can be wrong. A real server's parser is the
//! only unbiased reader of the bytes this client emits.
//!
//! **`#[ignore]` on purpose.** It needs the network and it reaches a third party, so it is not
//! part of `cargo test`. Run it deliberately:
//!
//! ```text
//! cargo test -p mail-runtime --test live_probe -- --ignored --nocapture
//! ```
//!
//! No credentials and no mailbox access: `CAPA` is answered before any authentication, which is
//! what makes this safe to keep. It sends one command and disconnects. If NTU ever objects, or
//! the host moves, delete it — the fake-server tests are what the suite actually depends on.

use mail_domain::Tls;
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{Transport, drive};
use tokio::sync::watch;

/// The host the NTU preset names for student and staff ids.
const HOST: &str = "msa.ntu.edu.tw";
const PORT: u16 = 995;

#[tokio::test]
#[ignore = "reaches a third-party server; run deliberately with --ignored"]
async fn the_ntu_preset_describes_the_server_it_claims_to() {
    let mut transport = match Transport::connect(HOST, PORT, Tls::Implicit).await {
        Ok(t) => t,
        // A test that fails because someone is on a train is a test people learn to ignore.
        Err(e) => {
            eprintln!("skipping: {HOST}:{PORT} unreachable ({e})");
            return;
        }
    };

    // Our own session, our own TLS, our own drive loop — only the server is someone else's.
    let mut session =
        Pop3Session::new("unused", "unused", vec![Pop3Command::Capa]).expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);

    let replies = drive(&mut session, &mut transport, &mut cancel)
        .await
        .expect("a real Dovecot accepts the bytes this client renders");

    let advertised: Vec<String> = replies
        .iter()
        .filter_map(|reply| match reply {
            mail_proto::Pop3Reply::Capabilities(atoms) => Some(atoms.clone()),
            _ => None,
        })
        .flatten()
        .map(|a| a.to_uppercase())
        .collect();
    eprintln!("{HOST} advertises: {advertised:?}");

    let has = |name: &str| advertised.iter().any(|a| a == name || a.starts_with(name));

    // The preset's `top: Supported::Yes` is the load-bearing one: `RETR` sets `\Seen` and `TOP`
    // does not, so a first sync without it marks the user's entire maildrop read in their
    // webmail. If this ever stops being advertised, the preset is wrong and so is the sync.
    assert!(has("TOP"), "the NTU preset claims TOP: {advertised:?}");
    assert!(
        has("UIDL"),
        "UIDL is how POP3 identity works at all: {advertised:?}"
    );
    assert!(
        has("PIPELINING"),
        "the preset claims pipelining: {advertised:?}"
    );
    // `sasl: vec![SaslMech::Plain]`, and the preset's comment that LOGIN is *not* offered —
    // which is why the auth prelude must not try it first.
    assert!(
        has("SASL"),
        "the preset authenticates with SASL PLAIN: {advertised:?}"
    );
}
