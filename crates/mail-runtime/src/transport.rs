//! The only place in the workspace that opens a socket.
//!
//! Everything below this crate returns [`IoNeed`] and gets fed [`IoReady`]; this module is what
//! turns those into syscalls. Keeping it small and boring is the point — the protocol machines
//! carry the complexity precisely so that this does not have to.

use crate::RuntimeError;
use mail_domain::Tls;
use mail_proto::{IoNeed, IoReady};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// How many bytes to ask the kernel for at once.
///
/// A whole IMAP `FETCH` of a large message arrives across many reads regardless, so this trades
/// syscalls against a per-connection buffer rather than trying to read a response in one go.
const READ_CHUNK: usize = 16 * 1024;

/// An open connection, plaintext or TLS.
///
/// Not a trait. There are exactly two cases and no third is planned; an enum keeps the match
/// exhaustive and costs nothing.
#[derive(Debug)]
enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    /// Held only while [`Transport::upgrade`] moves the socket into the TLS wrapper. Any use
    /// of a transport in this state is a bug, and every method says so rather than panicking.
    Upgrading,
}

/// A connection a [`mail_proto::Machine`] can be driven over.
#[derive(Debug)]
pub struct Transport {
    stream: Stream,
}

/// Choose the crypto provider, once, before rustls is asked to.
///
/// rustls 0.23 refuses to pick when more than one provider is compiled in, and **panics** rather
/// than returning an error. Two are compiled in here and neither is optional: this crate asks
/// for `ring` explicitly, while `reqwest` and `keyring` pull in `aws-lc-rs`. So every TLS
/// connection this program makes would have aborted the process on the line that built the
/// config.
///
/// Nothing caught it, because every test server in this repository listens on loopback with
/// `Tls::Plaintext` — the one setting no real account uses. The first attempt at a TLS
/// connection to a real host found it immediately.
///
/// `ring`, because that is what `mail-runtime` selected in `Cargo.toml`; a competing install
/// from elsewhere in the process is not an error, so the result is deliberately ignored.
fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl Transport {
    /// Connect, applying `tls` before returning.
    ///
    /// [`Tls::StartTlsRequired`] returns a plaintext transport: the caller sends the protocol's
    /// own upgrade command and then calls [`Transport::upgrade`]. There is deliberately no
    /// opportunistic mode — an upgrade that may silently not happen is one an attacker chooses
    /// for you.
    pub async fn connect(host: &str, port: u16, tls: Tls) -> Result<Self, RuntimeError> {
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(|e| RuntimeError::Connect(format!("{host}:{port}: {e}")))?;
        // Mail is request/response and latency-sensitive; waiting to coalesce a 20-byte command
        // with nothing else just adds a round trip.
        let _ = tcp.set_nodelay(true);

        let stream = match tls {
            Tls::Implicit => Stream::Tls(Box::new(Self::wrap(tcp, host).await?)),
            Tls::StartTlsRequired | Tls::Plaintext => Stream::Plain(tcp),
        };
        Ok(Self { stream })
    }

    /// Upgrade a plaintext connection after the protocol's `STARTTLS`/`STLS` was accepted.
    pub async fn upgrade(&mut self, host: &str) -> Result<(), RuntimeError> {
        match std::mem::replace(&mut self.stream, Stream::Upgrading) {
            Stream::Plain(tcp) => {
                self.stream = Stream::Tls(Box::new(Self::wrap(tcp, host).await?));
                Ok(())
            }
            // Upgrading twice is a bug in the caller, not a condition to tolerate silently.
            // Put the stream back so the error does not also destroy the connection.
            other => {
                self.stream = other;
                Err(RuntimeError::Tls("already upgraded".to_owned()))
            }
        }
    }

    async fn wrap(tcp: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, RuntimeError> {
        // Here rather than in `connect`, because `upgrade` builds a session too and this is
        // the one place that touches rustls at all.
        install_crypto_provider();
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        // No custom verifier and no way to disable verification. If a campus server presents a
        // chain webpki rejects, that is a conversation to have deliberately, not a flag to add.
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = rustls_pki_types::ServerName::try_from(host.to_owned())
            .map_err(|_| RuntimeError::Tls(format!("{host} is not a valid server name")))?;
        TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| RuntimeError::Tls(format!("{host}: {e}")))
    }

    /// Satisfy one [`IoNeed`].
    ///
    /// Returns `None` for needs that produce nothing to feed back — a write or a flush — so the
    /// caller keeps consuming its list rather than inventing an [`IoReady`] the machine never
    /// asked for.
    pub async fn satisfy(&mut self, need: &IoNeed) -> Result<Option<IoReady>, RuntimeError> {
        match need {
            IoNeed::Write(bytes) => {
                self.write_all(bytes).await?;
                Ok(None)
            }
            IoNeed::Flush => {
                self.flush().await?;
                Ok(None)
            }
            IoNeed::Read => {
                let mut buf = vec![0u8; READ_CHUNK];
                let n = self.read(&mut buf).await?;
                if n == 0 {
                    return Ok(Some(IoReady::Eof));
                }
                buf.truncate(n);
                Ok(Some(IoReady::Bytes(buf)))
            }
            IoNeed::Sleep(duration) => {
                tokio::time::sleep(*duration).await;
                Ok(Some(IoReady::Woke))
            }
            // The machine asks for TLS; the engine performs the upgrade, because it owns the
            // Transport by value and this method only has &mut self.
            IoNeed::OpenTls { .. } => Ok(None),
            IoNeed::Close => {
                let _ = self.flush().await;
                Ok(None)
            }
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        match &mut self.stream {
            Stream::Plain(s) => s.write_all(bytes).await,
            Stream::Tls(s) => s.write_all(bytes).await,
            Stream::Upgrading => {
                return Err(RuntimeError::Tls("transport used mid-upgrade".to_owned()));
            }
        }
        .map_err(|e| RuntimeError::Io(e.to_string()))
    }

    async fn flush(&mut self) -> Result<(), RuntimeError> {
        match &mut self.stream {
            Stream::Plain(s) => s.flush().await,
            Stream::Tls(s) => s.flush().await,
            Stream::Upgrading => {
                return Err(RuntimeError::Tls("transport used mid-upgrade".to_owned()));
            }
        }
        .map_err(|e| RuntimeError::Io(e.to_string()))
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, RuntimeError> {
        match &mut self.stream {
            Stream::Plain(s) => s.read(buf).await,
            Stream::Tls(s) => s.read(buf).await,
            Stream::Upgrading => {
                return Err(RuntimeError::Tls("transport used mid-upgrade".to_owned()));
            }
        }
        .map_err(|e| RuntimeError::Io(e.to_string()))
    }
}
