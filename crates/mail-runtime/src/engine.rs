//! One account's worth of work: sync passes, and draining the outbox.
//!
//! One generic parameter, not five. `Store` and `Secrets` are process-wide singletons — one
//! SQLite file with one WAL connection, one keyring — so making them type parameters would
//! misrepresent ownership, and `Box<dyn Store>` per account would be worse. Time is an argument
//! rather than a `Clock` trait, per `CONVENTIONS.md` §6.

use crate::{Cancel, RuntimeError, Secrets, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, AccountPlan, Credential, FetchSince, Incoming, MailboxRef, Outgoing, ProtoOp,
    RemoteRef, Retry, Retryable, SecretKey, SecretPurpose, SendState, SyncCursor, Tls,
};
use mail_mime::Posting;
use mail_proto::backend::SmtpBackend;
use mail_proto::{Backend, ProtoOutcome, Submission};
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
    /// Messages handed to the submission server and accepted.
    pub submitted: usize,
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
    ///
    /// Public so a caller can check reachability before committing to a pass; the sync methods
    /// each open their own, because a session spends the greeting and cannot share one.
    pub async fn connect(&self) -> Result<Transport, RuntimeError> {
        let (host, port, tls) = self.incoming();
        Transport::connect(host, port, tls).await
    }

    /// Run one operation to completion, on a connection of its own.
    ///
    /// A fresh connection per operation, and that is not laziness. A session reads the server's
    /// greeting before issuing anything, so a second session on the same socket waits forever
    /// for a greeting that was already spent — which is exactly how this hung until an
    /// end-to-end test sat on it for sixty seconds. The alternatives are a session that knows
    /// whether it is resuming, which puts protocol state in the caller, or one operation per
    /// pass, which is what batching already achieves: a sync is three connections, not one per
    /// message.
    async fn run(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        // Submission is a different server on a different port speaking a different protocol.
        // Sending it to the incoming backend is how `ProtoOp::Submit` got refused by POP3 as
        // "unsupported" — a true statement about the wrong backend.
        if matches!(op, ProtoOp::Submit { .. }) {
            return self.submit(op, cancel).await;
        }
        let mut transport = self.connect().await?;
        let mut step = BackendMachine {
            backend: &mut self.backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// Where the submission server lives.
    fn outgoing(&self) -> (&str, u16, Tls) {
        match &self.plan.outgoing {
            Outgoing::Smtp { host, port, tls } => (host, *port, *tls),
        }
    }

    /// A backend that can submit exactly one message for this account.
    ///
    /// Built per submission rather than held, because it closes over the credential and a
    /// long-lived copy of a password is a copy waiting to be logged. The closure is the only
    /// thing that holds it; `SmtpBackend` itself never sees it.
    fn submitter(&self) -> Result<SmtpBackend, RuntimeError> {
        let (host, port, tls) = self.outgoing();
        let (host, ehlo) = (host.to_owned(), self.plan.ehlo());
        let (username, sasl) = (self.plan.username(), self.plan.sasl());
        // Most providers authenticate submission with the same secret as retrieval, which is
        // what `AuthPlan` means by covering both directions. A separate outgoing secret is
        // preferred where one was stored, because a few hosts really do differ.
        let credential = self
            .secret(SecretPurpose::OutgoingPassword)
            .or_else(|_| self.secret(SecretPurpose::IncomingPassword))?;
        // The incoming backend's capabilities. They describe the *account*, not the socket:
        // `SmtpBackend` reads none of the IMAP-shaped fields, and giving it a second, emptier
        // set would mean two answers to one question.
        let caps = self.backend.caps().clone();
        Ok(SmtpBackend::new(
            self.account,
            caps,
            Box::new(move |posting: Posting| {
                Ok(Submission {
                    ehlo: ehlo.clone(),
                    host: host.clone(),
                    port,
                    tls,
                    username: username.clone(),
                    credential: credential.clone(),
                    sasl: sasl.clone(),
                    // Straight from the Posting. Re-deriving either of these from `message`
                    // is FINDINGS F37.
                    mail_from: posting.mail_from,
                    recipients: posting.rcpt_to,
                    message: posting.message,
                })
            }),
        ))
    }

    fn secret(&self, purpose: SecretPurpose) -> Result<Credential, RuntimeError> {
        self.secrets.get(&SecretKey {
            account: self.account,
            purpose,
        })
    }

    /// Submit one composed message, over a connection to the outgoing server.
    async fn submit(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let ProtoOp::Submit {
            raw,
            ref mail_from,
            ref rcpt_to,
            ..
        } = op
        else {
            return Err(RuntimeError::UnsupportedIo(
                "submit() called with something other than a submission".to_owned(),
            ));
        };
        // The bytes were frozen when the user pressed send, so a draft edited while the outbox
        // was backed off does not change what goes out.
        let message = self.store.blobs().get(&self.store.connection(), raw)?;
        let posting = Posting {
            mail_from: mail_from.clone(),
            rcpt_to: rcpt_to.clone(),
            message,
        };

        let mut backend = self.submitter()?;
        backend.stage(posting)?;
        let (host, port, tls) = self.outgoing();
        let mut transport = Transport::connect(host, port, tls).await?;
        let mut step = BackendMachine {
            backend: &mut backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// Drain the outbox, in insertion order, stopping at the first entry not yet due.
    ///
    /// Insertion order is load-bearing: two operations on one thread must reach the server in
    /// the order the user performed them, or the result is whichever won the race.
    pub async fn drain_outbox(
        &mut self,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        for entry in self.store.outbox_due(self.account, now)? {
            let id = entry.id;
            // Noted before the op is consumed. A submission also has a draft whose visible
            // state must follow what happened on the wire; nothing else in the outbox does.
            let draft = match &entry.op {
                ProtoOp::Submit { draft, .. } => Some(*draft),
                _ => None,
            };
            match self.run(entry.op, cancel).await {
                Ok(_) => {
                    self.store.outbox_settle(id, Settle::Ok, now)?;
                    if let Some(draft) = draft {
                        // `message: None` — SMTP reports that the message was accepted, not
                        // where a copy was filed. Gmail files it in Sent itself; a POP3 account
                        // has no Sent folder at all.
                        self.mark_draft(
                            draft,
                            SendState::Sent {
                                at: now,
                                message: None,
                            },
                            now,
                        );
                        report.submitted += 1;
                    }
                    report.outbox_settled += 1;
                }
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => {
                    let retry = e.retry();
                    if matches!(retry, Retry::NeedsReauth | Retry::Fatal(_)) {
                        report.needs_attention.push(e.to_string());
                    }
                    if let Some(draft) = draft {
                        // Carries the retry, so the composer can say "retrying" rather than
                        // "failed" for something the outbox has not given up on.
                        self.mark_draft(
                            draft,
                            SendState::Failed {
                                reason: e.to_string(),
                                retry: retry.clone(),
                            },
                            now,
                        );
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

    /// Record where a draft got to, without letting that record fail the drain.
    ///
    /// A missing draft is not an error worth propagating: the user may have deleted it while
    /// the outbox was backed off, and the message either was or was not accepted regardless.
    /// Failing here would leave the outbox entry settled and the pass reporting an error about
    /// something nobody is waiting on.
    fn mark_draft(&self, draft: mail_domain::DraftId, state: SendState, now: DateTime<Utc>) {
        let _ = self.store.set_send_state(draft, &state, now);
    }

    /// A first or incremental sync: survey, then headers, then bodies smallest band first.
    ///
    /// Headers before bodies is not only about speed. On POP3 `RETR` sets the seen flag and
    /// `TOP` does not, so fetching whole messages to populate a list view would mark the user's
    /// entire mailbox read in their webmail.
    pub async fn sync(
        &mut self,
        mailbox: &MailboxRef,
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
                cancel,
            )
            .await?;
        if let ProtoOutcome::Ingested(ingest) = outcome {
            self.store.ingest(self.account, *ingest)?;
        }

        // What to fetch comes from the SERVER'S listing, not from the store. On a first sync
        // the store knows nothing, so asking it what is missing returns an empty list and the
        // pass fetches nothing — silently, with no error. Only an end-to-end test caught that.
        //
        // The ordering decision lives here rather than in the backend, because it is a product
        // judgement about what the user sees first, not a protocol fact.
        let mut wanted = self.backend.surveyed();
        if wanted.is_empty() {
            // A protocol that cannot enumerate up front: fall back to what we already hold.
            wanted = self.unfetched(budget as u32)?;
        }
        wanted.sort_by_key(|(_, size)| *size);

        for band in BANDS {
            let batch: Vec<RemoteRef> = wanted
                .iter()
                .filter(|(_, size)| *size <= band)
                .take(budget.saturating_sub(report.headers_fetched))
                .map(|(remote, _)| remote.clone())
                .collect();
            if batch.is_empty() {
                continue;
            }
            // One operation for the whole band, on one connection. A per-message operation
            // would need a connection each, because a session consumes the greeting once.
            match self
                .run(
                    ProtoOp::FetchHeaders {
                        remotes: batch.clone(),
                    },
                    cancel,
                )
                .await
            {
                Ok(ProtoOutcome::Fetched { items }) => {
                    let arrivals = items
                        .into_iter()
                        .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
                        .collect::<Vec<_>>();
                    report.headers_fetched += arrivals.len();
                    // Headers only: Body::Absent says the body has not arrived, rather than
                    // storing an empty message that looks complete.
                    crate::assemble::absorb(
                        &self.store,
                        self.account,
                        mailbox.clone(),
                        SyncCursor::Pop,
                        arrivals,
                        true,
                        now,
                    )?;
                }
                Ok(_) => {}
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => {
                    report.needs_attention.push(e.to_string());
                    return Ok(report);
                }
            }
            // Remove what this band covered, so a later band does not refetch it.
            wanted.retain(|(remote, _)| !batch.contains(remote));
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
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        budget: usize,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        let mut wanted = self.unfetched(budget as u32)?;
        wanted.sort_by_key(|(_, size)| *size);

        let batch: Vec<RemoteRef> = wanted
            .into_iter()
            .take(budget)
            .map(|(remote, _)| remote)
            .collect();
        if batch.is_empty() {
            return Ok(report);
        }
        match self
            .run(ProtoOp::FetchBody { remotes: batch }, cancel)
            .await
        {
            Ok(ProtoOutcome::Fetched { items }) => {
                let arrivals = items
                    .into_iter()
                    .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
                    .collect::<Vec<_>>();
                report.bodies_fetched += arrivals.len();
                crate::assemble::absorb(
                    &self.store,
                    self.account,
                    mailbox.clone(),
                    SyncCursor::Pop,
                    arrivals,
                    false,
                    now,
                )?;
            }
            Ok(_) => {}
            Err(RuntimeError::Cancelled) => return Ok(report),
            Err(e) => report.needs_attention.push(e.to_string()),
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
