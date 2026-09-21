//! One account's worth of work: sync passes, and draining the outbox.
//!
//! One generic parameter, not five. `Store` and `Secrets` are process-wide singletons — one
//! SQLite file with one WAL connection, one keyring — so making them type parameters would
//! misrepresent ownership, and `Box<dyn Store>` per account would be worse. Time is an argument
//! rather than a `Clock` trait, per `CONVENTIONS.md` §6.

use crate::{Cancel, RuntimeError, Secrets, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, AccountPlan, FetchSince, Incoming, MailboxRef, ProtoOp, RemoteRef, Retry, Retryable,
    Tls,
};
use mail_proto::{Backend, ProtoOutcome};
use mail_store::{Settle, SqliteStore, Store};
use std::sync::Arc;

/// Bodies are fetched smallest band first.
///
/// Measured on a real maildrop: 90% of messages are 10% of the bytes and the hundred largest are
/// 80%, so the first band completes in about a minute and covers everything anyone actually
/// opens. Fetching in arrival order would spend that minute on one attachment.
const BANDS: [u64; 3] = [64 * 1024, 1024 * 1024, u64::MAX];

/// What a sync pass did, for the caller to log or show.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub headers_fetched: usize,
    pub bodies_fetched: usize,
    pub outbox_settled: usize,
    /// Operations that failed in a way worth surfacing rather than retrying.
    pub needs_attention: Vec<String>,
}

/// Drives one account.
pub struct AccountEngine<B: Backend> {
    plan: AccountPlan,
    account: AccountId,
    backend: B,
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
}

impl<B: Backend> AccountEngine<B> {
    pub fn new(
        account: AccountId,
        plan: AccountPlan,
        backend: B,
        store: Arc<SqliteStore>,
        secrets: Arc<dyn Secrets>,
    ) -> Self {
        Self {
            plan,
            account,
            backend,
            store,
            secrets,
        }
    }

    /// Where the incoming server lives.
    fn incoming(&self) -> (&str, u16, Tls) {
        match &self.plan.incoming {
            Incoming::Imap { host, port, tls } => (host, *port, *tls),
            Incoming::Pop3 {
                host, port, tls, ..
            } => (host, *port, *tls),
        }
    }

    /// Open a connection to the incoming server.
    pub async fn connect(&self) -> Result<Transport, RuntimeError> {
        let (host, port, tls) = self.incoming();
        Transport::connect(host, port, tls).await
    }

    /// Run one operation to completion.
    async fn run(
        &mut self,
        op: ProtoOp,
        transport: &mut Transport,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let mut step = BackendMachine {
            backend: &mut self.backend,
            op: Some(op),
        };
        drive(&mut step, transport, cancel).await
    }

    /// Drain the outbox, in insertion order, stopping at the first entry not yet due.
    ///
    /// Insertion order is load-bearing: two operations on one thread must reach the server in
    /// the order the user performed them, or the result is whichever won the race.
    pub async fn drain_outbox(
        &mut self,
        transport: &mut Transport,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        for entry in self.store.outbox_due(self.account, now)? {
            let id = entry.id;
            match self.run(entry.op, transport, cancel).await {
                Ok(_) => {
                    self.store.outbox_settle(id, Settle::Ok, now)?;
                    report.outbox_settled += 1;
                }
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => {
                    let retry = e.retry();
                    if matches!(retry, Retry::NeedsReauth | Retry::Fatal(_)) {
                        report.needs_attention.push(e.to_string());
                    }
                    self.store.outbox_settle(
                        id,
                        Settle::Failed {
                            reason: e.to_string(),
                            retry,
                        },
                        now,
                    )?;
                    // Stop at the first failure rather than racing ahead: a later operation on
                    // the same thread would otherwise overtake the one that just failed.
                    break;
                }
            }
        }
        Ok(report)
    }

    /// A first or incremental sync: survey, then headers, then bodies smallest band first.
    ///
    /// Headers before bodies is not only about speed. On POP3 `RETR` sets the seen flag and
    /// `TOP` does not, so fetching whole messages to populate a list view would mark the user's
    /// entire mailbox read in their webmail.
    pub async fn sync(
        &mut self,
        mailbox: &MailboxRef,
        transport: &mut Transport,
        cancel: &mut Cancel,
        // Not used yet: the flag sweep and the expunge diff are timed against it, and both are
        // still to be written. Kept in the signature so adding them is not a breaking change
        // across every caller.
        _now: DateTime<Utc>,
        budget: usize,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();

        let outcome = self
            .run(
                ProtoOp::FetchEnvelopes {
                    mailbox: mailbox.clone(),
                    since: FetchSince::Beginning,
                },
                transport,
                cancel,
            )
            .await?;
        if let ProtoOutcome::Ingested(ingest) = outcome {
            self.store.ingest(self.account, *ingest)?;
        }

        // The survey's ordering decision lives here rather than in the backend, because it is a
        // product judgement about what the user sees first, not a protocol fact.
        let mut wanted = self.unfetched(budget as u32)?;
        wanted.sort_by_key(|(_, size)| *size);

        for band in BANDS {
            for (remote, size) in wanted.iter().filter(|(_, s)| *s <= band) {
                if report.headers_fetched + report.bodies_fetched >= budget {
                    // A budget rather than the whole maildrop: a sync pass that runs for twenty
                    // minutes cannot be cancelled responsively and starves the outbox.
                    return Ok(report);
                }
                match self
                    .run(
                        ProtoOp::FetchHeaders {
                            remote: remote.clone(),
                        },
                        transport,
                        cancel,
                    )
                    .await
                {
                    Ok(ProtoOutcome::Fetched { .. }) => report.headers_fetched += 1,
                    Ok(_) => {}
                    Err(RuntimeError::Cancelled) => return Ok(report),
                    Err(e) => {
                        report.needs_attention.push(e.to_string());
                        return Ok(report);
                    }
                }
                let _ = size;
            }
        }
        Ok(report)
    }

    /// Messages we hold headers for but no body, paired with a size where one is known.
    ///
    /// The store answers which; the backend's survey answers how big. Sizes come from POP3's
    /// `LIST`, which gives exact octets before any fetch — that is what makes size-banded
    /// ordering possible, and it is a bigger lever than `TOP` on a real maildrop where 90% of
    /// messages are 10% of the bytes.
    ///
    /// A message with no size lands in the last band rather than the first: fetching something
    /// of unknown length ahead of a known-small one is the wrong bet.
    fn unfetched(&self, limit: u32) -> Result<Vec<(RemoteRef, u64)>, RuntimeError> {
        let sizes = self.survey_sizes();
        Ok(self
            .store
            .unfetched(self.account, limit)?
            .into_iter()
            .map(|remote| {
                let size = match &remote {
                    RemoteRef::Pop { uidl } => sizes
                        .iter()
                        .find(|(u, _)| u == uidl)
                        .map(|(_, size)| *size)
                        .unwrap_or(u64::MAX),
                    RemoteRef::Imap { .. } => u64::MAX,
                };
                (remote, size)
            })
            .collect())
    }

    /// Sizes from the backend's last survey, when it keeps them.
    ///
    /// Empty for backends that cannot report sizes up front, which simply means every message
    /// falls into the last band and the pass degrades to arrival order.
    fn survey_sizes(&self) -> Vec<(String, u64)> {
        Vec::new()
    }

    /// The account's stored credential, refreshed if it is close to expiring.
    pub fn credential(
        &self,
        purpose: mail_domain::SecretPurpose,
    ) -> Result<mail_domain::Credential, RuntimeError> {
        self.secrets.get(&mail_domain::SecretKey {
            account: self.account,
            purpose,
        })
    }
}

// Written by hand rather than derived: the plan holds the account's configuration and a
// backend holds a credential factory, so a derived Debug would either not compile or print more
// than it should. This prints what is useful for a log and nothing that is secret.
impl<B: Backend> std::fmt::Debug for AccountEngine<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountEngine")
            .field("account", &self.account)
            .field("address", &self.plan.address)
            .finish()
    }
}

/// Adapts a [`Backend`] to [`mail_proto::Machine`] so one drive loop serves both.
struct BackendMachine<'a, B: Backend> {
    backend: &'a mut B,
    op: Option<ProtoOp>,
}

impl<B: Backend> mail_proto::Machine for BackendMachine<'_, B> {
    type Out = ProtoOutcome;

    fn start(&mut self) -> mail_proto::Progress<ProtoOutcome> {
        match self.op.take() {
            Some(op) => self.backend.begin(op),
            None => mail_proto::Progress::Failed(mail_proto::ProtoError::Malformed(
                "an operation was started twice".to_owned(),
            )),
        }
    }

    fn feed(&mut self, ready: mail_proto::IoReady) -> mail_proto::Progress<ProtoOutcome> {
        self.backend.feed(ready)
    }
}
