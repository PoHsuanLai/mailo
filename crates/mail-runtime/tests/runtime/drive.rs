//! The drive loop against a real socket, and against a silent one in memory.
//!
//! A loopback `TcpListener` rather than a mock: the thing most likely to be wrong here is how
//! the loop behaves when bytes arrive in pieces or a peer hangs up, and a mock transport would
//! only assert my own assumptions back at me.

use mail_domain::{Retry, Retryable, Tls};
use mail_net::Transport;
use mail_proto::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use mail_runtime::drive::{QUIET, WIND_DOWN};
use mail_runtime::{RuntimeError, drive};
use porter_client::AuthenticatedStream;
use porter_core::stream::{ByteStream, DuplexEnd, duplex};
use std::time::Duration;
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

/// A connection held in memory, and the server's end of it for the test to speak through.
///
/// On a paused clock, which a loopback socket does not mix with: nothing here waits on the
/// kernel, so the only thing that moves time is the deadline under test.
fn in_memory(capacity: usize) -> (Transport, DuplexEnd) {
    let (ours, theirs) = duplex(capacity);
    let transport = Transport::relayed(AuthenticatedStream::Memory(ours)).expect("in memory");
    (transport, theirs)
}

/// A server that greets and then never says another word: a NAT that forgot the connection, or
/// a provider that dropped it without a reset. Nothing tells that apart from a slow server but
/// the clock, so after [`QUIET`] the exchange is called stalled, as an error a reconnect answers,
/// rather than parked on for ever.
#[tokio::test(start_paused = true)]
async fn a_server_gone_silent_is_stalled_not_waited_on() {
    let (mut transport, mut server) = in_memory(64 * 1024);
    ByteStream::write_all(&mut server, b"+OK ready\r\n")
        .await
        .unwrap();
    let (_tx, mut cancel) = watch::channel(false);
    let began = tokio::time::Instant::now();

    let err = drive(&mut Echo::default(), &mut transport, &mut cancel)
        .await
        .expect_err("a server that never answers does not complete a session");

    assert!(
        matches!(err, RuntimeError::Stalled { after, .. } if after == QUIET),
        "{err:?}"
    );
    let waited = began.elapsed();
    assert!(
        waited >= QUIET && waited < QUIET + Duration::from_secs(1),
        "{waited:?}"
    );
    assert!(
        matches!(err.retry(), Retry::After(_)),
        "a stalled connection is answered by a new one: {:?}",
        err.retry()
    );
    // Kept open to the end: a dropped end is an EOF, which is the other failure.
    drop(server);
}

/// The wait the deadline is given back to: a server that speaks, however slowly, is not
/// stalled. Silence just short of [`QUIET`] between each line still completes.
#[tokio::test(start_paused = true)]
async fn a_slow_server_is_not_a_stalled_one() {
    let (mut transport, mut server) = in_memory(64 * 1024);
    let speaking = tokio::spawn(async move {
        let nearly = QUIET - Duration::from_secs(1);
        tokio::time::sleep(nearly).await;
        ByteStream::write_all(&mut server, b"+OK ready\r\n")
            .await
            .unwrap();
        tokio::time::sleep(nearly).await;
        ByteStream::write_all(&mut server, b"+OK done\r\n")
            .await
            .unwrap();
        server
    });
    let (_tx, mut cancel) = watch::channel(false);
    let out = drive(&mut Echo::default(), &mut transport, &mut cancel)
        .await
        .expect("a slow server is still a server");
    assert_eq!(out, "+OK done");
    drop(speaking.await.unwrap());
}

/// The wind-down after an interrupt is bounded too. A peer that stopped reading leaves `DONE`
/// stuck behind a full buffer, and a cancellation that waited on it would be a hang of its own.
#[tokio::test(start_paused = true)]
async fn the_wind_down_after_an_interrupt_is_bounded() {
    // Room for one byte: `DONE` cannot be written unless the server reads, and it never does.
    let (mut transport, mut server) = in_memory(1);
    tokio::spawn(async move {
        ByteStream::write_all(&mut server, b"+OK IDLE\r\n")
            .await
            .unwrap();
        // Holds its end open and reads nothing, for ever.
        std::future::pending::<()>().await;
        drop(server);
    });
    let (tx, mut cancel) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = tx.send(true);
    });

    let err = drive(&mut Echo::default(), &mut transport, &mut cancel)
        .await
        .expect_err("a wind-down that cannot be written does not complete");
    assert!(
        matches!(err, RuntimeError::Stalled { after, .. } if after == WIND_DOWN),
        "{err:?}"
    );
}
