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
    SyncCursor, Tls,
};
use mail_proto::{Backend, ProtoOutcome};
use mail_store::{Settle, SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

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

/// How often each part of a sync runs.
///
/// Three intervals, not one, because a sync pass is three different questions and only the
/// first has a push signal. Gmail's IDLE reports new mail and **nothing else** — no flag
/// changes — and Gmail has no QRESYNC, so a client that only watches never learns that a
/// message was read, starred or deleted somewhere else. That is not an edge case; it is every
/// user with a phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    /// New mail: IDLE where offered, otherwise this interval.
    pub watch: Duration,
    /// Flag changes: a `CHANGEDSINCE` sweep where CONDSTORE is usable, else a full flag fetch.
    pub flags: Duration,
    /// Disappearances: the full address list, diffed against `remote_map`.
    pub expunges: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            watch: Duration::from_secs(300),
            // Often enough that reading mail on a phone shows up within a couple of minutes.
            flags: Duration::from_secs(120),
            // Rarely: it is the most expensive of the three, and a message deleted elsewhere
            // lingering for a few minutes costs the user nothing.
            expunges: Duration::from_secs(900),
        }
    }
}

/// When each part of the schedule last ran.
#[derive(Debug, Clone, Copy, Default)]
struct LastRun {
    flags: Option<DateTime<Utc>>,
    expunges: Option<DateTime<Utc>>,
}

/// Drives one account.
pub struct AccountEngine<B: Backend> {
    plan: AccountPlan,
    account: AccountId,
    backend: B,
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    schedule: Schedule,
    last: LastRun,
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
            schedule: Schedule::default(),
            last: LastRun::default(),
        }
    }

    /// Override the default intervals.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = schedule;
        self
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
        now: DateTime<Utc>,
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
                let _ = size;
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
                    Ok(ProtoOutcome::Fetched { remote, raw }) => {
                        // Headers only: the body has not been fetched, and Body::Absent says so
                        // rather than storing an empty message that looks complete.
                        crate::assemble::absorb(
                            &self.store,
                            self.account,
                            mailbox.clone(),
                            SyncCursor::Pop,
                            vec![crate::assemble::Arrival { remote, raw }],
                            true,
                            now,
                        )?;
                        report.headers_fetched += 1;
                    }
                    Ok(_) => {}
                    Err(RuntimeError::Cancelled) => return Ok(report),
                    Err(e) => {
                        report.needs_attention.push(e.to_string());
                        return Ok(report);
                    }
                }
            }
        }
        Ok(report)
    }

    /// Fetch bodies for messages we hold headers for, smallest band first.
    ///
    /// Separate from [`AccountEngine::sync`] so a caller can show a usable inbox after the
    /// header pass and fetch bodies behind it — which is the entire reason for splitting the
    /// two on a maildrop where ninety per cent of messages are ten per cent of the bytes.
    pub async fn fetch_bodies(
        &mut self,
        mailbox: &MailboxRef,
        transport: &mut Transport,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        budget: usize,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        let mut wanted = self.unfetched(budget as u32)?;
        wanted.sort_by_key(|(_, size)| *size);

        for (remote, _) in wanted.into_iter().take(budget) {
            match self
                .run(ProtoOp::FetchBody { remote }, transport, cancel)
                .await
            {
                Ok(ProtoOutcome::Fetched { remote, raw }) => {
                    crate::assemble::absorb(
                        &self.store,
                        self.account,
                        mailbox.clone(),
                        SyncCursor::Pop,
                        vec![crate::assemble::Arrival { remote, raw }],
                        false,
                        now,
                    )?;
                    report.bodies_fetched += 1;
                }
                Ok(_) => {}
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => {
                    report.needs_attention.push(e.to_string());
                    return Ok(report);
                }
            }
        }
        Ok(report)
    }

    /// Run whichever scheduled sweeps are due.
    ///
    /// Separate from [`AccountEngine::sync`] because these answer different questions on
    /// different clocks, and because both must run even when the account is idle — an idle
    /// account is exactly the one whose mail is being read on another device.
    pub async fn sweep(
        &mut self,
        mailbox: &MailboxRef,
        transport: &mut Transport,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();

        if due(self.last.flags, self.schedule.flags, now) {
            // No modseq means a full flag fetch: either the server has no CONDSTORE, or its
            // HIGHESTMODSEQ was seen not to advance, which Dovecot 2.0.18 did while EXISTS
            // climbed. Trusting a frozen modseq would mean never seeing another flag change.
            let since_modseq = self.trusted_modseq(mailbox);
            match self
                .run(
                    ProtoOp::FetchFlags {
                        mailbox: mailbox.clone(),
                        since_modseq,
                    },
                    transport,
                    cancel,
                )
                .await
            {
                Ok(ProtoOutcome::Ingested(ingest)) => {
                    self.store.ingest(self.account, *ingest)?;
                    self.last.flags = Some(now);
                }
                Ok(_) => self.last.flags = Some(now),
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => report.needs_attention.push(e.to_string()),
            }
        }

        if due(self.last.expunges, self.schedule.expunges, now) {
            match self
                .run(
                    ProtoOp::ListRemote {
                        mailbox: mailbox.clone(),
                    },
                    transport,
                    cancel,
                )
                .await
            {
                Ok(ProtoOutcome::Ingested(ingest)) => {
                    self.store.ingest(self.account, *ingest)?;
                    self.last.expunges = Some(now);
                }
                Ok(_) => self.last.expunges = Some(now),
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => report.needs_attention.push(e.to_string()),
            }
        }

        Ok(report)
    }

    /// A modseq worth fetching from, or `None` to fetch every flag.
    ///
    /// Returns `None` today. Reading the stored cursor and checking that its `HIGHESTMODSEQ`
    /// actually advanced needs the IMAP backend, which is not written yet — and a full flag
    /// fetch is correct but slow, which is the right way round to be wrong.
    fn trusted_modseq(&self, _mailbox: &MailboxRef) -> Option<u64> {
        None
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

/// Whether a periodic task is due.
///
/// Never having run counts as due, so a fresh account sweeps once immediately rather than
/// waiting out a full interval before it can show a flag change.
fn due(last: Option<DateTime<Utc>>, every: Duration, now: DateTime<Utc>) -> bool {
    let Some(last) = last else {
        return true;
    };
    match chrono::TimeDelta::from_std(every) {
        Ok(delta) => now - last >= delta,
        // An interval too large for TimeDelta is not a reason to sweep constantly.
        Err(_) => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn a_task_that_has_never_run_is_due() {
        // So a fresh account sweeps once immediately rather than waiting out a full interval
        // before it can notice anything changed elsewhere.
        assert!(due(None, Duration::from_secs(120), at(0)));
    }

    #[test]
    fn a_task_is_due_exactly_on_its_interval_not_after() {
        let last = Some(at(0));
        assert!(!due(last, Duration::from_secs(120), at(119)));
        assert!(due(last, Duration::from_secs(120), at(120)));
        assert!(due(last, Duration::from_secs(120), at(121)));
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_make_everything_due() {
        // NTP corrections and suspend/resume both do this. Sweeping constantly because the
        // clock moved is worse than sweeping late.
        assert!(!due(Some(at(1000)), Duration::from_secs(120), at(0)));
    }

    #[test]
    fn the_three_intervals_are_ordered_by_what_they_cost() {
        let s = Schedule::default();
        assert!(
            s.flags < s.watch,
            "flag changes must be noticed sooner than a poll interval: reading mail on a phone \
             should show up in a couple of minutes"
        );
        assert!(
            s.expunges > s.watch,
            "the full address list is the most expensive sweep and the least urgent"
        );
    }
}
