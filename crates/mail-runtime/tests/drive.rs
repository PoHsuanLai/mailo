//! The drive loop against a real socket.
//!
//! A loopback `TcpListener` rather than a mock: the thing most likely to be wrong here is how
//! the loop behaves when bytes arrive in pieces or a peer hangs up, and a mock transport would
//! only assert my own assumptions back at me.

use mail_domain::Tls;
use mail_proto::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use mail_runtime::{Transport, drive};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Greet, send one command, read one reply. A real session's shape without a protocol.
#[derive(Default)]
struct Echo {
    seen: Vec<u8>,
    greeted: bool,
}

impl Machine for Echo {
    type Out = String;

    fn start(&mut self) -> Progress<String> {
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<String> {
        match ready {
            // The whole point of Interrupt: wind down in protocol, do not just vanish.
            IoReady::Interrupt => Progress::Need(vec![IoNeed::Write(b"DONE\r\n".to_vec())]),
            IoReady::Eof => Progress::Failed(ProtoError::UnexpectedEof),
            IoReady::Woke | IoReady::TlsOpen => Progress::Need(vec![IoNeed::Read]),
            IoReady::Bytes(bytes) => {
                self.seen.extend_from_slice(&bytes);
                if !self.seen.ends_with(b"\r\n") {
                    return Progress::Need(vec![IoNeed::Read]);
                }
                let line = String::from_utf8_lossy(&self.seen).trim_end().to_owned();
                self.seen.clear();
                if !self.greeted {
                    self.greeted = true;
                    if line.contains("IDLE") {
                        // Parked: only an Interrupt or more bytes moves this.
                        return Progress::Need(vec![IoNeed::Read]);
                    }
                    return Progress::Need(vec![
                        IoNeed::Write(b"HELLO\r\n".to_vec()),
                        IoNeed::Read,
                    ]);
                }
                if line.starts_with("ERR") {
                    return Progress::Failed(ProtoError::Refused {
                        kind: Refusal::Permanent,
                        text: line,
                    });
                }
                Progress::Done(line)
            }
        }
    }
}

/// Serve `script` to one client, returning whatever the client wrote back.
async fn serve(script: Vec<&'static [u8]>) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut got = Vec::new();
        for chunk in script {
            sock.write_all(chunk).await.unwrap();
            sock.flush().await.unwrap();
            let mut buf = [0u8; 1024];
            // No read is normal for a server that speaks twice in a row.
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_millis(120), sock.read(&mut buf))
                    .await
            {
                got.extend_from_slice(&buf[..n]);
            }
        }
        got
    });
    (port, handle)
}

async fn connect(port: u16) -> Transport {
    Transport::connect("127.0.0.1", port, Tls::Plaintext)
        .await
        .expect("loopback connect")
}

#[tokio::test]
async fn a_session_runs_to_completion_over_a_real_socket() {
    let (port, server) = serve(vec![b"+OK ready\r\n", b"+OK done\r\n"]).await;
    let mut transport = connect(port).await;
    let (_tx, mut cancel) = watch::channel(false);

    let out = drive(&mut Echo::default(), &mut transport, &mut cancel)
        .await
        .expect("should complete");
    assert_eq!(out, "+OK done");
    assert!(
        String::from_utf8_lossy(&server.await.unwrap()).contains("HELLO"),
        "the command should have reached the server"
    );
}

/// The case a mock would never catch: a reply split across TCP segments.
#[tokio::test]
async fn a_reply_split_across_segments_is_reassembled() {
    let (port, _server) = serve(vec![b"+OK rea", b"dy\r\n", b"+OK do", b"ne\r\n"]).await;
    let mut transport = connect(port).await;
    let (_tx, mut cancel) = watch::channel(false);
    assert_eq!(
        drive(&mut Echo::default(), &mut transport, &mut cancel)
            .await
            .expect("should complete"),
        "+OK done"
    );
}

/// A peer that hangs up mid-session is an error, not a hang.
#[tokio::test]
async fn a_closed_connection_ends_the_session() {
    let (port, _server) = serve(vec![b"+OK ready\r\n"]).await;
    let mut transport = connect(port).await;
    let (_tx, mut cancel) = watch::channel(false);
    let err = drive(&mut Echo::default(), &mut transport, &mut cancel)
        .await
        .expect_err("the server closes without replying");
    let text = format!("{err}");
    // Named exactly. This read `contains("closed") || contains("io")`, and "io" is a substring
    // of "connection" — so the disjunction passed on any error mentioning a connection at all,
    // including ones that had nothing to do with the peer hanging up.
    assert!(
        text.contains("connection closed unexpectedly"),
        "a closed peer should report EOF, not {text:?}"
    );
}

/// Cancellation reaches a machine parked waiting for bytes, and it answers first.
///
/// This is why `IoReady::Interrupt` exists. Archiving a thread while an IDLE is outstanding must
/// not tear the connection out from under a half-finished command.
#[tokio::test]
async fn cancelling_interrupts_a_parked_machine_and_lets_it_send_done() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        sock.write_all(b"+OK IDLE\r\n").await.unwrap();
        // Then say nothing, exactly as a server holding an IDLE does.
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), sock.read(&mut buf))
            .await
            .map(|r| r.unwrap_or(0))
            .unwrap_or(0);
        buf[..n].to_vec()
    });

    let mut transport = connect(port).await;
    let (tx, mut cancel) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let _ = tx.send(true);
    });

    let err = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        drive(&mut Echo::default(), &mut transport, &mut cancel),
    )
    .await
    .expect("cancellation must not hang")
    .expect_err("a cancelled session does not complete");
    assert!(format!("{err}").contains("cancelled"), "{err}");

    assert_eq!(
        String::from_utf8_lossy(&server.await.unwrap()).trim_end(),
        "DONE",
        "the machine must wind down in protocol before the loop gives up"
    );
}

/// A TLS handshake must not abort the process.
///
/// rustls 0.23 panics — not errors — when more than one crypto provider is compiled in and none
/// has been installed. Two are compiled in here and neither is optional: this crate selects
/// `ring`, while `reqwest` and `keyring` bring `aws-lc-rs`. So `mailo sync` aborted on its first
/// TLS connection, which is every connection to a real account.
///
/// Nothing in the suite caught it, because every fake server here listens on loopback with
/// `Tls::Plaintext` — the one setting no real account uses. This asserts the failure is an
/// ordinary connection error rather than a panic, using a port nothing is listening on: the
/// provider is installed while building the session, before the socket matters.
#[tokio::test]
async fn a_tls_connection_reports_an_error_rather_than_aborting() {
    // The listener must *accept*, or this never reaches rustls at all and proves nothing — the
    // first version of this test connected to a closed port, failed at TCP, and passed happily
    // with the provider install removed.
    //
    // It accepts and then says nothing, so the handshake fails on its own terms. What is being
    // asserted is that the failure arrives as a `Result`.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            // Held so the peer sees an open connection rather than a reset.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            drop(sock);
        }
    });

    let outcome = Transport::connect("127.0.0.1", port, Tls::Implicit).await;
    assert!(
        outcome.is_err(),
        "a server that never speaks TLS should be an error, not a session"
    );
}
