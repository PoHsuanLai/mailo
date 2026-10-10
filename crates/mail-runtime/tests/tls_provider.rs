// Its own test binary: the regression shows only where no rustls provider is installed yet.
//! A TLS handshake against a real socket, in a process of its own.
//!
//! Split from the drive loop's tests (`tests/runtime/drive.rs`) when the rest were folded into one
//! binary: once any test in the process has installed a default `CryptoProvider`, this proves
//! nothing.

use mail_domain::Tls;
use mail_runtime::Transport;
use tokio::net::TcpListener;

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
