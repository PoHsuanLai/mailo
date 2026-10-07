//! porter's own relay, in front of a fake server on loopback, as the accountd link.
//!
//! What an app is handed by `Accounts::open_authenticated` is the end of a stream whose other end
//! is `porter_proxy::relay`: it dials the endpoint, signs in with the password it was given and
//! greets the app with `* PREAUTH` (IMAP), `220` and an `EHLO` reply without `AUTH` (SMTP), `+OK
//! porter relay ready` (POP3) or the capability list without `SASL` (ManageSieve). That relay is
//! the real one, so what these tests check is what mailo does with what accountd's relay says.
//! Only the bus is missing: [`Relays::open`] hands the app end over in memory, as
//! `InProcess` does for an app hosting porter.
//!
//! A tap sits between the app and the relay and keeps the bytes the *app* sent: the relay answers
//! a login the app attempts anyway ("a client that logs in anyway is told it succeeded"), so the
//! server's transcript cannot show that mailo did not try.

#![allow(dead_code)]

use mail_runtime::Transport;
use mail_runtime::link::{Accountd, Answer, Changes, LinkError};
use porter_client::AuthenticatedStream;
use porter_core::stream::{ByteStream, DuplexEnd, duplex};
use porter_core::{
    AccountId, Audience, Candidate, CapabilityKind, GrantId, IssuedToken, Origin, RelayAuth,
    RelayPlan, SecretText, ServiceEndpoint, Tls,
};
use porter_proxy::{Connect, ConnectFault, TokioStream, relay};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;

/// Dials loopback in plain text, which is all a fake server speaks.
pub struct Loopback;

impl Connect for Loopback {
    type Stream = TokioStream<TcpStream>;

    async fn dial(&self, origin: &Origin, _tls: Tls) -> Result<Self::Stream, ConnectFault> {
        TcpStream::connect((origin.host.as_str(), origin.port))
            .await
            .map(TokioStream)
            .map_err(|_| ConnectFault::Unreachable)
    }

    async fn upgrade(
        &self,
        _stream: Self::Stream,
        _host: &str,
    ) -> Result<Self::Stream, ConnectFault> {
        Err(ConnectFault::Tls)
    }
}

/// accountd, as far as the engines ask: relays for the endpoints of one account, and nothing else.
#[derive(Debug)]
pub struct Relays {
    password: String,
    app_sent: Arc<Mutex<Vec<u8>>>,
    opened: AtomicUsize,
    tokens: Mutex<Vec<IssuedToken>>,
}

impl Relays {
    /// Relays that sign in with `password` (the account's, which only accountd has).
    pub fn signing_in_with(password: &str) -> Arc<Relays> {
        Arc::new(Relays {
            password: password.to_owned(),
            app_sent: Arc::new(Mutex::new(Vec::new())),
            opened: AtomicUsize::new(0),
            tokens: Mutex::new(Vec::new()),
        })
    }

    /// Every byte the app sent into a relay, in order, across connections.
    pub fn app_sent(&self) -> String {
        String::from_utf8_lossy(&self.app_sent.lock().unwrap()).into_owned()
    }

    /// How many relays were opened.
    pub fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }

    /// Answers for `token`, last first.
    pub fn issuing(&self, token: IssuedToken) {
        self.tokens.lock().unwrap().push(token);
    }
}

fn refuse<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
}

/// Carries bytes both ways between the app and the relay, keeping what the app sent.
async fn tap(mut near: DuplexEnd, mut far: DuplexEnd, sent: Arc<Mutex<Vec<u8>>>) {
    let (mut from_app, mut from_relay) = (vec![0u8; 8192], vec![0u8; 8192]);
    loop {
        tokio::select! {
            got = ByteStream::read(&mut near, &mut from_app) => match got {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    sent.lock().unwrap().extend_from_slice(&from_app[..n]);
                    if ByteStream::write_all(&mut far, &from_app[..n]).await.is_err() {
                        break;
                    }
                }
            },
            got = ByteStream::read(&mut far, &mut from_relay) => match got {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ByteStream::write_all(&mut near, &from_relay[..n]).await.is_err() {
                        break;
                    }
                }
            },
        }
    }
    let _ = ByteStream::shutdown(&mut far).await;
    let _ = ByteStream::shutdown(&mut near).await;
}

impl Accountd for Relays {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        refuse()
    }

    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        let next = self.tokens.lock().unwrap().pop();
        Box::pin(async move { next.ok_or(LinkError::Unreachable) })
    }

    fn open<'a>(
        &'a self,
        _grant: &'a GrantId,
        endpoint: &'a ServiceEndpoint,
    ) -> Answer<'a, Transport> {
        Box::pin(async move {
            self.opened.fetch_add(1, Ordering::SeqCst);
            // app <-> tap <-> relay
            let (app, near) = duplex(64 * 1024);
            let (far, relay_end) = duplex(64 * 1024);
            let plan = RelayPlan {
                endpoint: endpoint.clone(),
                kind: CapabilityKind::Mail,
                auth: RelayAuth::Password(SecretText::new(self.password.clone())),
            };
            tokio::spawn(async move {
                let _ = relay(plan, relay_end, &Loopback).await;
            });
            tokio::spawn(tap(near, far, self.app_sent.clone()));
            Transport::relayed(AuthenticatedStream::Memory(app))
                .map_err(|e| LinkError::Other(e.to_string()))
        })
    }

    fn add_account(&self) -> Answer<'_, AccountId> {
        refuse()
    }

    fn reauthenticate<'a>(&'a self, _: &'a AccountId) -> Answer<'a, ()> {
        refuse()
    }

    fn request_grant(&self) -> Answer<'_, Candidate> {
        refuse()
    }

    fn revoke<'a>(&'a self, _: &'a GrantId) -> Answer<'a, ()> {
        refuse()
    }

    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        Box::pin(async { Ok(None) })
    }
}
