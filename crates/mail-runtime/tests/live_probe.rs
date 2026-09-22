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
//! with the server named in the environment:
//!
//! ```text
//! MAILO_LIVE_POP3=pop.example.edu[:995] cargo test -p mail-runtime --test live_probe -- --ignored --nocapture
//! ```
//!
//! No credentials and no mailbox access: `CAPA` is answered before any authentication, which is
//! what makes this safe to keep. It sends one command and disconnects. With no server named it
//! skips — the fake-server tests are what the suite actually depends on.

use mail_domain::Tls;
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{Transport, drive};
use tokio::sync::watch;

/// `HOST[:PORT]` of a POP3 server with implicit TLS; 995 when no port is given.
const SERVER: &str = "MAILO_LIVE_POP3";

#[tokio::test]
#[ignore = "reaches a third-party server; run deliberately with --ignored"]
async fn a_real_pop3_server_accepts_what_this_client_sends() {
    let Ok(server) = std::env::var(SERVER) else {
        eprintln!("skipping: set {SERVER}=HOST[:PORT] to name a POP3 server");
        return;
    };
    let (host, port) = match server.rsplit_once(':') {
        Some((host, port)) => (
            host.to_owned(),
            port.parse()
                .unwrap_or_else(|_| panic!("{SERVER}: {port:?} is not a port")),
        ),
        None => (server.clone(), 995),
    };
    let mut transport = match Transport::connect(&host, port, Tls::Implicit).await {
        Ok(t) => t,
        // A test that fails because someone is on a train is a test people learn to ignore.
        Err(e) => {
            eprintln!("skipping: {host}:{port} unreachable ({e})");
            return;
        }
    };

    // Our own session, our own TLS, our own drive loop — only the server is someone else's.
    let mut session =
        Pop3Session::new("unused", "unused", vec![Pop3Command::Capa]).expect("a clean credential");
    let (_tx, mut cancel) = watch::channel(false);

    let replies = drive(&mut session, &mut transport, &mut cancel)
        .await
        .expect("a real server accepts the bytes this client renders");

    let advertised: Vec<String> = replies
        .iter()
        .filter_map(|reply| match reply {
            mail_proto::Pop3Reply::Capabilities(atoms) => Some(atoms.clone()),
            _ => None,
        })
        .flatten()
        .map(|a| a.to_uppercase())
        .collect();
    eprintln!("{host} advertises: {advertised:?}");

    let has = |name: &str| advertised.iter().any(|a| a == name || a.starts_with(name));

    assert!(
        has("UIDL"),
        "UIDL is how POP3 identity works at all: {advertised:?}"
    );
    // `RETR` sets `\Seen` on servers that also serve IMAP, and `TOP` does not, so without `TOP`
    // a first sync would mark the user's whole maildrop read in their webmail. The backend
    // refuses rather than do that; this says in advance whether it will have to.
    assert!(
        has("TOP"),
        "without TOP this client will not fetch headers: {advertised:?}"
    );
}
