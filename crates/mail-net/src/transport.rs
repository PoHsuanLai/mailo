//! The only place in the workspace that opens a socket.
//!
//! Everything below this crate returns [`IoNeed`] and gets fed [`IoReady`]; this module is what
//! turns those into syscalls. Keeping it small and boring is the point — the protocol machines
//! carry the complexity precisely so that this does not have to.

use crate::NetError;
use crate::deflate::Deflate;
use crate::error::{Failure, within};
use mail_domain::Tls;
use mail_proto::{IoNeed, IoReady};
use porter_client::AuthenticatedStream;
use porter_core::stream::{ByteStream, DuplexEnd};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// How many bytes to ask the kernel for at once.
///
/// A whole IMAP `FETCH` of a large message arrives across many reads regardless, so this trades
/// syscalls against a per-connection buffer rather than trying to read a response in one go.
const READ_CHUNK: usize = 16 * 1024;

/// The longest a connection may take to open: the TCP handshake, and the TLS one after it, each.
///
/// A server that is up answers both in well under a second; one that has not answered in this
/// long is not there, and the attempt fails as [`NetError::Stalled`] rather than waiting on
/// the kernel's own limit, which is minutes.
pub const OPENING: Duration = Duration::from_secs(30);

/// How long a connection sits quiet before the kernel checks that the other end is still there.
///
/// A held IDLE says nothing for many minutes by design, and a NAT that forgets a quiet mapping,
/// or a provider that drops one without a word, leaves a socket that looks open and never
/// speaks again. A probe a minute keeps the mapping warm and finds a dead end within a few.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(60);

/// An open connection, plaintext or TLS, or a porter relay's stream.
///
/// Not a trait. There are a few cases and no more planned; an enum keeps the match exhaustive and
/// costs nothing.
#[derive(Debug)]
enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    /// A porter relay, as a Unix socket it handed over: the relay dialled the server, secured
    /// the connection and signed in before this end saw a byte ([`Transport::relayed`]).
    #[cfg(unix)]
    Relay(tokio::net::UnixStream),
    /// A relay hosted in this process, as an in-memory duplex.
    Memory(DuplexEnd),
    /// Held only while [`Transport::upgrade`] moves the socket into the TLS wrapper. Any use
    /// of a transport in this state is a bug, and every method says so rather than panicking.
    Upgrading,
}

/// A connection a [`mail_proto::Machine`] can be driven over.
#[derive(Debug)]
pub struct Transport {
    stream: Stream,
    /// Set once `COMPRESS DEFLATE` was accepted ([`IoNeed::Deflate`]); from then on everything
    /// either way goes through it.
    deflate: Option<Box<Deflate>>,
    /// Inflated bytes a read has not yet handed on, when one chunk from the server held more
    /// than the caller's buffer.
    inflated: Vec<u8>,
}

const RELAYED: &str = "a relay's connection is already secured by the relay";

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

/// Have the kernel probe `tcp` when it has been quiet for [`KEEPALIVE_IDLE`].
///
/// Best effort: a system that refuses the option still has the read deadlines in
/// [`crate::drive`], so the failure is only logged.
fn keep_alive(tcp: &TcpStream) {
    let probes = socket2::TcpKeepalive::new().with_time(KEEPALIVE_IDLE);
    // Fifteen seconds between probes once probing has begun. Not settable everywhere; where it
    // is not, the system's own applies.
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "windows",
    ))]
    let probes = probes.with_interval(Duration::from_secs(15));
    if let Err(e) = socket2::SockRef::from(tcp).set_tcp_keepalive(&probes) {
        log::debug!("TCP keepalive could not be set: {e}");
    }
}

impl Transport {
    fn of(stream: Stream) -> Self {
        Self {
            stream,
            deflate: None,
            inflated: Vec::new(),
        }
    }

    /// Connect, applying `tls` before returning.
    ///
    /// [`Tls::StartTlsRequired`] returns a plaintext transport: the caller sends the protocol's
    /// own upgrade command and then calls [`Transport::upgrade`]. There is deliberately no
    /// opportunistic mode — an upgrade that may silently not happen is one an attacker chooses
    /// for you.
    pub async fn connect(host: &str, port: u16, tls: Tls) -> Result<Self, NetError> {
        let tcp = within(OPENING, "connecting", async {
            TcpStream::connect((host, port))
                .await
                .map_err(|e| NetError::Connect(Failure::new(format!("{host}:{port}"), e)))
        })
        .await?;
        // Mail is request/response and latency-sensitive; waiting to coalesce a 20-byte command
        // with nothing else just adds a round trip.
        let _ = tcp.set_nodelay(true);
        keep_alive(&tcp);

        let stream = match tls {
            Tls::Implicit => Stream::Tls(Box::new(Self::wrap(tcp, host).await?)),
            Tls::StartTlsRequired | Tls::Plaintext => Stream::Plain(tcp),
        };
        Ok(Self::of(stream))
    }

    /// A connection that is already open and already authenticated: the end of a porter relay
    /// (`Accounts::open_authenticated`). The relay owns TLS and the sign-in, so [`Transport::upgrade`]
    /// has nothing to do on it, and the machine driven over it must not authenticate (the IMAP
    /// session without a `LOGIN`, the SMTP and ManageSieve ones `relayed`).
    pub fn relayed(stream: AuthenticatedStream) -> Result<Self, NetError> {
        let stream = match stream {
            #[cfg(unix)]
            AuthenticatedStream::Fd(fd) => {
                let socket = std::os::unix::net::UnixStream::from(fd);
                socket
                    .set_nonblocking(true)
                    .map_err(|e| NetError::Io(Failure::new("the relay's socket", e)))?;
                let socket = tokio::net::UnixStream::from_std(socket)
                    .map_err(|e| NetError::Io(Failure::new("the relay's socket", e)))?;
                Stream::Relay(socket)
            }
            AuthenticatedStream::Memory(end) => Stream::Memory(end),
        };
        Ok(Self::of(stream))
    }

    /// Upgrade a plaintext connection after the protocol's `STARTTLS`/`STLS` was accepted.
    pub async fn upgrade(&mut self, host: &str) -> Result<(), NetError> {
        match std::mem::replace(&mut self.stream, Stream::Upgrading) {
            Stream::Plain(tcp) => {
                self.stream = Stream::Tls(Box::new(Self::wrap(tcp, host).await?));
                Ok(())
            }
            // A relay's stream is as secure as the relay made it, and a machine over it does not
            // ask: if one does, say so rather than pretend.
            #[cfg(unix)]
            other @ Stream::Relay(_) => {
                self.stream = other;
                Err(NetError::Tls(Failure::said(RELAYED)))
            }
            other @ Stream::Memory(_) => {
                self.stream = other;
                Err(NetError::Tls(Failure::said(RELAYED)))
            }
            // Upgrading twice is a bug in the caller, not a condition to tolerate silently.
            // Put the stream back so the error does not also destroy the connection.
            other => {
                self.stream = other;
                Err(NetError::Tls(Failure::said("already upgraded")))
            }
        }
    }

    pub async fn wrap(tcp: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, NetError> {
        // Here rather than in `connect`, because `upgrade` builds a session too and this is
        // the one place that touches rustls at all.
        install_crypto_provider();
        // The bundled roots and the system's (`crate::roots`). No custom verifier and no way to
        // disable verification: a campus server's authority is believed when the system has it.
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(crate::roots::server_roots())
            .with_no_client_auth();
        let name = rustls_pki_types::ServerName::try_from(host.to_owned()).map_err(|_| {
            NetError::Tls(Failure::said(format!("{host} is not a valid server name")))
        })?;
        within(OPENING, "securing the connection", async {
            TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|e| NetError::Tls(Failure::new(host.to_string(), e)))
        })
        .await
    }

    /// Satisfy one [`IoNeed`].
    ///
    /// Returns `None` for needs that produce nothing to feed back — a write or a flush — so the
    /// caller keeps consuming its list rather than inventing an [`IoReady`] the machine never
    /// asked for.
    pub async fn satisfy(&mut self, need: &IoNeed) -> Result<Option<IoReady>, NetError> {
        match need {
            IoNeed::Write(bytes) => {
                self.write_all(bytes).await?;
                Ok(None)
            }
            IoNeed::Deflate { leftover } => {
                let mut deflate = Box::new(Deflate::new());
                // Read with the `OK`, so not on the socket any more.
                if !leftover.is_empty() {
                    self.inflated = deflate
                        .inflate(leftover)
                        .map_err(|e| NetError::Io(Failure::caused(e)))?;
                }
                self.deflate = Some(deflate);
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

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), NetError> {
        match &mut self.deflate {
            Some(deflate) => {
                let squeezed = deflate
                    .compress(bytes)
                    .map_err(|e| NetError::Io(Failure::caused(e)))?;
                self.write_raw(&squeezed).await
            }
            None => self.write_raw(bytes).await,
        }
    }

    async fn write_raw(&mut self, bytes: &[u8]) -> Result<(), NetError> {
        match &mut self.stream {
            Stream::Plain(s) => s.write_all(bytes).await,
            Stream::Tls(s) => s.write_all(bytes).await,
            #[cfg(unix)]
            Stream::Relay(s) => s.write_all(bytes).await,
            Stream::Memory(s) => ByteStream::write_all(s, bytes).await,
            Stream::Upgrading => {
                return Err(NetError::Tls(Failure::said("transport used mid-upgrade")));
            }
        }
        .map_err(|e| NetError::Io(Failure::caused(e)))
    }

    async fn flush(&mut self) -> Result<(), NetError> {
        match &mut self.stream {
            Stream::Plain(s) => s.flush().await,
            Stream::Tls(s) => s.flush().await,
            #[cfg(unix)]
            Stream::Relay(s) => s.flush().await,
            Stream::Memory(_) => Ok(()),
            Stream::Upgrading => {
                return Err(NetError::Tls(Failure::said("transport used mid-upgrade")));
            }
        }
        .map_err(|e| NetError::Io(Failure::caused(e)))
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        if self.deflate.is_none() {
            return self.read_raw(buf).await;
        }
        // Until something inflates to at least a byte: a read may end inside a compressed block.
        while self.inflated.is_empty() {
            let mut raw = vec![0u8; READ_CHUNK];
            let n = self.read_raw(&mut raw).await?;
            if n == 0 {
                return Ok(0);
            }
            if let Some(deflate) = &mut self.deflate {
                self.inflated = deflate
                    .inflate(&raw[..n])
                    .map_err(|e| NetError::Io(Failure::caused(e)))?;
            }
        }
        let n = buf.len().min(self.inflated.len());
        buf[..n].copy_from_slice(&self.inflated[..n]);
        self.inflated.drain(..n);
        Ok(n)
    }

    async fn read_raw(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        match &mut self.stream {
            Stream::Plain(s) => s.read(buf).await,
            Stream::Tls(s) => s.read(buf).await,
            #[cfg(unix)]
            Stream::Relay(s) => s.read(buf).await,
            Stream::Memory(s) => ByteStream::read(s, buf).await,
            Stream::Upgrading => {
                return Err(NetError::Tls(Failure::said("transport used mid-upgrade")));
            }
        }
        .map_err(|e| NetError::Io(Failure::caused(e)))
    }
}
