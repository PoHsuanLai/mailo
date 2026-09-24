//! One account's worth of work: sync passes, and draining the outbox.
//!
//! One generic parameter, not five. `Store` and `Secrets` are process-wide singletons — one
//! SQLite file with one WAL connection, one keyring — so making them type parameters would
//! misrepresent ownership, and `Box<dyn Store>` per account would be worse. Time is an argument
//! rather than a `Clock` trait, per `CONVENTIONS.md` §6.

use crate::renewal::{AfterRefusal, Renewal, Token};
use crate::{Cancel, RuntimeError, Secrets, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, AccountPlan, BlobId, Condstore, Credential, FetchSince, Incoming,
    MailboxRef, MailboxRole, MessageId, Outgoing, PartTree, ProtoOp, RemoteRef, Resync, Retry,
    Retryable, SecretKey, SecretPurpose, SendState, SyncCursor, SystemFlag, Tls, UidValidity,
    WatchMode,
};
use mail_mime::Posting;
use mail_proto::backend::SmtpBackend;
use mail_proto::{Backend, ProtoOutcome, Submission};
use mail_store::{Settle, SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

mod wait;
pub use wait::Woke;

/// Bodies are fetched smallest band first.
///
/// Measured on a real maildrop: 90% of messages are 10% of the bytes and the hundred largest are
/// 80%, so the first band completes in about a minute and covers everything anyone actually
/// opens. Fetching in arrival order would spend that minute on one attachment.
const BANDS: [u64; 3] = [64 * 1024, 1024 * 1024, u64::MAX];

/// Put `wanted` in fetch order: smallest band first, and within a band the order it came in.
///
/// Stable on purpose. Callers pass messages newest first, and a sort that broke ties by size
/// would undo that inside every band — a 3 KB newsletter from 2019 ahead of this morning's 4 KB
/// reply, on a first sync the user is watching.
/// Above this, a large IMAP message is fetched as its text, and its attachments wait on the server
/// until someone opens one.
///
/// The second band's edge. Below it `BODY.PEEK[]` is a single round trip and F113's measurement
/// stands — asking for structure costs more than it saves. Above it, one attachment is most of
/// the bytes, and the structure is a few hundred.
const PARTED_ABOVE: u64 = BANDS[1];

/// Which leaves of a large message to download with it.
///
/// Every part that could be the body — text, not declared an attachment — whatever its size,
/// because the reader needs it and a stand-in could be taken for it. And anything small enough
/// for the first band, which is mostly inline images: fetching one later costs a round trip to
/// save a few kilobytes.
fn keep_now(node: &PartTree) -> bool {
    match node {
        PartTree::Leaf {
            mime,
            octets,
            attachment,
            ..
        } => (!attachment && mime.starts_with("text/")) || *octets <= BANDS[0],
        PartTree::Multipart { .. } => true,
    }
}

fn by_band(wanted: &mut [(RemoteRef, u64)]) {
    wanted.sort_by_key(|(_, size)| BANDS.iter().position(|&band| *size <= band));
}

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
    /// Still queued when the pass ended, waiting on a retry.
    ///
    /// Separate from `needs_attention` because it is not trouble: a refused connection backs off
    /// and tries again, and interrupting the user about it would train them to ignore the
    /// warnings that matter. But it is not *nothing* either — someone who has just run `send`
    /// and then `sync` will otherwise read "0 sent" as success and believe their mail has gone.
    pub still_queued: usize,
    /// The server rejected the credential, on any account in this pass.
    ///
    /// Separate from `needs_attention`, which is prose for a person, because something has to
    /// *act* on it: a rejected password retried every five minutes is two hundred and eighty
    /// failed logins a day against the user's own mail server, which is how an account gets
    /// locked. A poll loop must stop on this and wait for the user, not back off and continue.
    pub needs_reauth: bool,
    /// The longest wait a server asked for during this pass, where one asked.
    ///
    /// A rate limit is the one refusal whose entire remedy is doing nothing, and hammering
    /// through it lengthens the lockout — so the number the server named (or the hour
    /// `Retry::After` supplies when it names none) has to outlive the pass that saw it. Kept
    /// apart from `needs_reauth` because they call for opposite things: stop and ask the user,
    /// versus come back later unaided.
    pub hold: Option<std::time::Duration>,
    /// Messages this pass stored for the first time, in the order they were stored.
    ///
    /// Identities, not copies: by the time anyone reads this the same pass has applied the
    /// server's flags and may have fetched bodies, so what a caller wants to know about a message
    /// — is it still unread, is it still in the inbox — is the store's current answer, not the
    /// one the header fetch built. A message already held that arrives again (a remap, a body
    /// filling in, the same mail under a second UID) is not here: it did not *arrive*.
    pub arrived: Vec<MessageId>,
    /// Messages uploaded into a mailbox by this pass's outbox (`ProtoOp::Append`).
    pub appended: usize,
}

/// The messages a patch stored for the first time.
///
/// Only meaningful for the patch of a header fetch: that path upserts a message only when its
/// key is new, since a header carries no body to fill in. A body fetch's patch upserts messages
/// already held, and must not be read with this.
fn first_stored(patch: &mail_domain::Patch) -> impl Iterator<Item = MessageId> + '_ {
    patch.changes.iter().filter_map(|change| match change {
        mail_domain::Change::MessageUpsert(message) => Some(message.id),
        _ => None,
    })
}

impl SyncReport {
    /// Fold one operation's verdict into the pass.
    ///
    /// The two facts a caller has to *act* on, gathered in one place so the several sites that
    /// see a `Retry` cannot disagree about which ones matter. `Retry::Now` and `Retry::Fatal`
    /// are decided within the pass — reconnect, or give up on that operation — and leave nothing
    /// for the caller to schedule.
    pub fn saw(&mut self, retry: &Retry) {
        match retry {
            Retry::NeedsReauth => self.needs_reauth = true,
            // The longest wait anyone asked for. Ordinary backoffs land here too, at a minute or
            // so, and are absorbed by the caller's own interval; a rate limit is an hour and is
            // not. Both mean the same thing to a scheduler, so neither needs a special case.
            Retry::After(wait) => self.hold = Some(self.hold.map_or(*wait, |had| had.max(*wait))),
            Retry::Now | Retry::Fatal(_) => {}
        }
    }
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
    /// How often a waiting watch looks in the outbox for a send that has come due.
    ///
    /// Local and cheap — one indexed query, no network — and the bound on how late a send
    /// scheduled by *another* process leaves: the watch only knows the times that were queued
    /// when it began waiting, and this is how it notices the rest.
    pub outbox: Duration,
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
            outbox: Duration::from_secs(15),
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
    /// Where [`Outgoing::Graph`] posts. Graph's own address, except under test.
    graph_url: String,
    /// Keeps an OAuth account's tokens fresh between and within passes. `None` for a password
    /// account, and for an engine nobody gave one — which then signs in with what it was built
    /// with, as every engine did before a watch had to outlive an access token.
    renewal: Option<Renewal>,
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
            graph_url: crate::graph::SEND_MAIL.to_owned(),
            renewal: None,
        }
    }

    /// Renew the account's access tokens as they near expiry, and once after a refusal.
    ///
    /// The backend's session factory must read its credential from `renewal.held()`, or the
    /// renewed token never reaches a connection.
    pub fn with_renewal(mut self, renewal: Renewal) -> Self {
        self.renewal = Some(renewal);
        self
    }

    /// Send [`Outgoing::Graph`] mail somewhere other than Graph: a test's own listener.
    pub fn with_graph_url(mut self, url: impl Into<String>) -> Self {
        self.graph_url = url.into();
        self
    }

    /// Override the default intervals.
    pub fn with_schedule(mut self, schedule: Schedule) -> Self {
        self.schedule = schedule;
        self
    }

    /// Where the incoming server lives. `None` for an account that keeps its mail here.
    fn incoming(&self) -> Option<(&str, u16, Tls)> {
        match &self.plan.incoming {
            Incoming::Imap { host, port, tls } => Some((host, *port, *tls)),
            Incoming::Pop3 {
                host, port, tls, ..
            } => Some((host, *port, *tls)),
            Incoming::Local => None,
        }
    }

    /// Open a connection to the incoming server.
    ///
    /// Public so a caller can check reachability before committing to a pass; the sync methods
    /// each open their own, because a session spends the greeting and cannot share one.
    pub async fn connect(&self) -> Result<Transport, RuntimeError> {
        let (host, port, tls) = self
            .incoming()
            .ok_or(RuntimeError::NoServer("connect to"))?;
        Transport::connect(host, port, tls).await
    }

    /// Run one operation, with a token that is fresh — and, if the server refuses a token that
    /// looked valid, once more with a new one.
    ///
    /// Here rather than in each operation, because every one of them presents a token and a watch
    /// reaches all of them an hour in. A token can be refused before its expiry says it should
    /// be: revoked sessions, a clock that is wrong, an issuer that shortened a lifetime. One
    /// renewal answers that; a second would not, which is why [`Renewal::after_refusal`] will not
    /// renew a token it minted for this reason already.
    async fn run(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        self.run_leaving(op, cancel, None).await
    }

    /// [`Self::run`], with the moment a submission leaves: its `Date` is set to `leaving` on
    /// the way out. `None` for everything that is not a submission.
    async fn run_leaving(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
        leaving: Option<DateTime<Utc>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let Some(renewal) = &self.renewal else {
            return self.run_once(op, cancel, leaving).await;
        };
        let token = match op {
            ProtoOp::Submit { .. } => Token::Sending,
            _ => Token::Incoming,
        };
        renewal.ahead(token).await?;
        let first = self.run_once(op.clone(), cancel, leaving).await;
        let refused = matches!(&first, Err(e) if matches!(e.retry(), Retry::NeedsReauth));
        let Some(renewal) = self.renewal.as_ref().filter(|_| refused) else {
            return first;
        };
        match renewal.after_refusal(token).await? {
            AfterRefusal::TryAgain => self.run_once(op, cancel, leaving).await,
            AfterRefusal::StillRefused => first,
        }
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
    async fn run_once(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
        leaving: Option<DateTime<Utc>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        // Submission is a different server on a different port speaking a different protocol.
        // Sending it to the incoming backend is how `ProtoOp::Submit` got refused by POP3 as
        // "unsupported" — a true statement about the wrong backend.
        if matches!(op, ProtoOp::Submit { .. }) {
            return self.submit(op, cancel, leaving).await;
        }
        // An upload's bytes are a blob, and reading one is this crate's to do. Staged here, per
        // attempt, rather than by the caller: a retry after a renewed sign-in runs this again,
        // and bytes staged once would already have been spent by the first attempt.
        if let ProtoOp::Append { raw, .. } = &op {
            let bytes = self.store.blobs().get(&self.store.connection(), *raw)?;
            self.backend.stage_append(bytes);
        }
        let mut transport = self.connect().await?;
        let mut step = BackendMachine {
            backend: &mut self.backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// Where the submission server lives, for an account that submits over SMTP.
    fn outgoing(&self) -> Option<(&str, u16, Tls)> {
        match &self.plan.outgoing {
            Outgoing::Smtp { host, port, tls } => Some((host, *port, *tls)),
            Outgoing::Graph | Outgoing::Nowhere => None,
        }
    }

    /// A backend that can submit exactly one message for this account.
    ///
    /// Built per submission rather than held, because it closes over the credential and a
    /// long-lived copy of a password is a copy waiting to be logged. The closure is the only
    /// thing that holds it; `SmtpBackend` itself never sees it.
    fn submitter(&self) -> Result<SmtpBackend, RuntimeError> {
        let (host, port, tls) = self.outgoing().ok_or_else(|| {
            RuntimeError::UnsupportedIo("this account does not submit over SMTP".to_owned())
        })?;
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
                    receipt: None,
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
    ///
    /// `leaving` replaces the frozen `Date`, so a message held in the outbox — scheduled, or
    /// waiting out a lid that was shut — says when it left rather than when Send was pressed.
    /// Only that one field changes (`mail_mime::restamp`), and it is safe to change because this
    /// client signs nothing: there is no DKIM signature over the header for it to break.
    async fn submit(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
        leaving: Option<DateTime<Utc>>,
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
        if self.plan.outgoing == Outgoing::Nowhere {
            return Err(RuntimeError::NoServer("send from"));
        }
        // The bytes were frozen when the user pressed send, so a draft edited while the outbox
        // was backed off does not change what goes out.
        let frozen = self.store.blobs().get(&self.store.connection(), raw)?;
        let message = match leaving {
            Some(at) => mail_mime::restamp(&frozen, at),
            None => frozen,
        };
        let Some((host, port, tls)) = self.outgoing() else {
            return self.submit_to_graph(&message, rcpt_to).await;
        };
        let (host, port, tls) = (host.to_owned(), port, tls);
        let posting = Posting {
            mail_from: mail_from.clone(),
            rcpt_to: rcpt_to.clone(),
            message,
        };

        let mut backend = self.submitter()?;
        backend.stage(posting)?;
        let mut transport = Transport::connect(&host, port, tls).await?;
        let mut step = BackendMachine {
            backend: &mut backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// Submit through Microsoft Graph, for an account whose plan says [`Outgoing::Graph`].
    ///
    /// The token is the account's *outgoing* credential: a Graph access token, which the caller
    /// minted from the sign-in's refresh token before the pass (`signin::graph_token`). The
    /// incoming one is for Exchange's IMAP and Graph refuses it. Graph files the sent copy
    /// itself, and does not say where.
    async fn submit_to_graph(
        &self,
        message: &[u8],
        rcpt_to: &[String],
    ) -> Result<ProtoOutcome, RuntimeError> {
        let credential = self.secret(mail_domain::SecretPurpose::OutgoingPassword)?;
        let mail_domain::Credential::OAuth { access, .. } = credential else {
            return Err(RuntimeError::Secrets(
                "sending through Graph needs a Microsoft sign-in, not a password".to_owned(),
            ));
        };
        let http = crate::signin::http_client()?;
        crate::graph::send_mime(&http, &self.graph_url, &access, message, rcpt_to).await?;
        Ok(ProtoOutcome::Submitted { remote: None })
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
            let upload = match &entry.op {
                ProtoOp::Append {
                    mailbox,
                    flags,
                    raw,
                    ..
                } => Some((mailbox.clone(), flags.clone(), *raw)),
                _ => None,
            };
            if let Some(draft) = draft {
                // From here it is on its way and can no longer be taken back: `unsend` refuses a
                // draft that says so. Nothing set this before, so a send could be "cancelled"
                // while the server was already accepting it.
                self.mark_draft(draft, SendState::Sending, now);
            }
            // Stamped with the same instant the draft records as `Sent { at }`, so the two agree.
            match self.run_leaving(entry.op, cancel, draft.map(|_| now)).await {
                Ok(outcome) => {
                    self.store.outbox_settle(id, Settle::Ok, now)?;
                    if let (Some(upload), ProtoOutcome::Appended { remote }) = (upload, outcome) {
                        report.appended += 1;
                        // The upload happened whatever this says. Failing the drain over the
                        // local copy would report as lost a message the server now holds.
                        if let Err(e) = self.keep_appended(upload, remote, now) {
                            report.needs_attention.push(format!(
                                "uploaded, but not kept here until the next sync: {e}"
                            ));
                        }
                    }
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
                    report.saw(&retry);
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
        // Whatever is left, however it got there: a failure that backed off, or an entry whose
        // turn had not come. Counted after the loop rather than inside it, so a `break` on the
        // first failure does not undercount the rest.
        report.still_queued = self
            .store
            .outbox_due(
                self.account,
                now + chrono::TimeDelta::try_days(365).unwrap_or_default(),
            )
            .map(|due| due.len())
            .unwrap_or(0);
        Ok(report)
    }

    /// Keep a message this client just uploaded, so it is here before any sync fetches it.
    ///
    /// At the address the server gave it where it gave one (`APPENDUID`); the next sync then
    /// finds that UID already mapped and fetches nothing. Where it gave none the message is kept
    /// with no address, by identity, and the sync that finds it on the server maps it rather
    /// than storing it twice. Either way re-importing the same file finds it already held and
    /// uploads nothing.
    fn keep_appended(
        &self,
        (mailbox, flags, raw): (MailboxRef, Vec<SystemFlag>, mail_domain::BlobId),
        remote: Option<RemoteRef>,
        now: DateTime<Utc>,
    ) -> Result<(), RuntimeError> {
        let bytes = self.store.blobs().get(&self.store.connection(), raw)?;
        let role = self.role_of(&mailbox);
        match remote {
            Some(remote) => {
                crate::assemble::appended(
                    &self.store,
                    self.account,
                    crate::assemble::Destination { mailbox, role },
                    remote,
                    bytes,
                    &flags,
                    now,
                )?;
            }
            None => {
                let placement = mail_mime::archive::Placement::new(role, flags, Vec::new());
                crate::assemble::keep(
                    &self.store,
                    self.account,
                    vec![crate::assemble::Keep {
                        raw: bytes,
                        placement,
                    }],
                    now,
                )?;
            }
        }
        Ok(())
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

    /// Ask the server what it supports, and write down the answer.
    ///
    /// Capabilities are **discovered**, not configured — that is why `AccountCaps` is a separate
    /// type from `AccountPlan`, and why it carries `observed_at`. Until this existed the only
    /// writer was `account add`, storing the preset's *expectation*, and nothing ever replaced
    /// it: every account ran for ever on a guess. CONDSTORE could not be found, `MOVE` could not
    /// be found, and the `SPECIAL-USE` folder roles stayed empty, so three features that were
    /// fully implemented were unreachable at once.
    ///
    /// Two walks, because they answer different halves. The first result is written before the
    /// second runs: they are separate connections, and a `LIST` that fails afterwards should not
    /// discard what `CAPABILITY` already established.
    pub async fn refresh_caps(
        &mut self,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<AccountCaps, RuntimeError> {
        if let ProtoOutcome::Caps(caps) = self.run(ProtoOp::FetchCaps, cancel).await? {
            self.store.put_caps(self.account, &caps, now)?;
        }
        // Folder roles, where the protocol has folders at all. A backend without them answers
        // something other than `Caps`, and this falls back to what the backend already believes
        // rather than treating it as a failure.
        let outcome = self.run(ProtoOp::ListFolders, cancel).await?;
        let caps = match outcome {
            ProtoOutcome::Folders { caps, listed } => {
                self.store.put_folders(self.account, listed)?;
                *caps
            }
            ProtoOutcome::Caps(caps) => *caps,
            _ => self.backend.caps().clone(),
        };
        self.store.put_caps(self.account, &caps, now)?;
        Ok(caps)
    }

    /// Ask the server for its folders, and write down the answer.
    ///
    /// The second half of [`AccountEngine::refresh_caps`] on its own, for when the list is
    /// wanted and the capabilities are not stale. The roles come with it, as they always have.
    pub async fn refresh_folders(
        &mut self,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<Vec<mail_domain::Folder>, RuntimeError> {
        // A protocol without folders answers something else, and that is not a failure.
        if let ProtoOutcome::Folders { caps, listed } =
            self.run(ProtoOp::ListFolders, cancel).await?
        {
            self.store.put_folders(self.account, listed)?;
            self.store.put_caps(self.account, &caps, now)?;
        }
        Ok(self.store.folders(self.account)?)
    }

    /// Whether this account has folders and none have been listed yet.
    ///
    /// Capabilities are re-read daily, and the folder list with them. An account added this
    /// morning would otherwise show no folders until tomorrow, and refuse to rename or delete
    /// any, so a sync pass asks as soon as it finds the list empty.
    pub fn folders_unlisted(&self) -> bool {
        matches!(self.plan.incoming, Incoming::Imap { .. })
            && self
                .store
                .folders(self.account)
                .map(|f| f.is_empty())
                .unwrap_or(false)
    }

    /// Upload a draft into the server's Drafts folder.
    ///
    /// Without this a draft exists on one machine. That is the difference between a mail client
    /// and a mail client you can start a reply on and finish from a phone, and it is what
    /// `ProtoOp::Append` was written for — it simply had no caller (FINDINGS F57).
    ///
    /// The folder comes from the server's own `SPECIAL-USE` reply where there is one. An account
    /// whose capabilities name no Drafts folder is not an error and not a guess: uploading into
    /// a path nobody confirmed is how a message lands somewhere the user will never look, so
    /// this reports that there was nowhere to put it and leaves the draft local.
    pub async fn upload_draft(
        &mut self,
        draft: &mail_domain::Draft,
        raw: Vec<u8>,
        cancel: &mut Cancel,
    ) -> Result<bool, RuntimeError> {
        let Some(path) = self.folder_for(MailboxRole::Drafts) else {
            return Ok(false);
        };
        let _ = draft;
        // Stored, so the op names bytes that exist: `run_once` reads them back to stage them.
        let raw = self.store.blobs().put(&self.store.connection(), &raw)?;
        let op = ProtoOp::Append {
            mailbox: MailboxRef {
                account: self.account,
                path,
            },
            flags: vec![SystemFlag::Draft, SystemFlag::Seen],
            date: None,
            raw,
        };
        self.run(op, cancel).await?;
        Ok(true)
    }

    /// The server's path for a role, where it told us one.
    fn folder_for(&self, role: MailboxRole) -> Option<String> {
        self.backend
            .caps()
            .folders
            .0
            .iter()
            .find(|(_, known)| *known == role)
            .map(|(path, _)| path.clone())
    }

    /// Wait until the server has something new, or the caller interrupts.
    ///
    /// `IDLE` where the server offers it, which is the difference between mail appearing when it
    /// arrives and mail appearing up to five minutes later. Where it does not, this returns
    /// immediately and the caller falls back to its own interval — `WatchMode::Poll` is not a
    /// worse kind of watching, it is the absence of watching, and pretending otherwise by
    /// sleeping in here would hide the distinction from whoever has to schedule around it.
    ///
    /// Returns whether the server actually signalled. `false` means "no push available", so a
    /// caller can tell "nothing happened yet" from "do not wait on me".
    ///
    /// Cancellation is the whole reason `IoReady::Interrupt` exists: an `IDLE` with no traffic
    /// parks for as long as the server allows, so without a way in from outside, quitting the
    /// application would block on a socket that is behaving perfectly.
    pub async fn watch(
        &mut self,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
    ) -> Result<bool, RuntimeError> {
        if !matches!(self.backend.caps().watch, WatchMode::Idle) {
            return Ok(false);
        }
        // What this client has already synced, so mail that landed between the last pass and
        // this `IDLE` wakes the watch at once instead of waiting for the next, unrelated push.
        let uidnext = match self.store.cursor(mailbox) {
            Ok(Some(SyncCursor::Imap { uidnext, .. })) => Some(uidnext),
            _ => None,
        };
        match self
            .run(
                ProtoOp::Watch {
                    mailbox: mailbox.clone(),
                    uidnext,
                },
                cancel,
            )
            .await
        {
            Ok(ProtoOutcome::Woken) => Ok(true),
            // Any other completion is a server that answered something unexpected rather than
            // an error worth stopping for; the caller's interval still applies.
            Ok(_) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Whether what we believe about the server is old enough to be worth re-asking.
    ///
    /// A server gains and loses extensions across upgrades, and an account moved between
    /// providers keeps its row. Daily is often enough to notice, and rare enough to cost
    /// nothing; re-reading on every pass would be a wasted round trip every few minutes.
    pub fn caps_are_stale(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.backend.caps().observed_at)
            > chrono::TimeDelta::try_hours(24).expect("24h is in range")
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
        if let ProtoOutcome::Ingested(mut ingest) = outcome {
            // The backend cannot answer this: it is sans-I/O and has never seen what we stored.
            // It reports what the server said; comparing that against the last sync is the
            // runtime's job, and getting it wrong means every stored UID addresses a different
            // message than we think it does.
            if let Some(fresh) = &ingest.cursor {
                ingest.validity = UidValidity::between(self.store.cursor(mailbox)?.as_ref(), fresh);
            }
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
            // A protocol that cannot enumerate up front: fall back to what we already hold,
            // which the store hands back newest first.
            wanted = self.unfetched(budget as u32)?;
        } else {
            // Minus what is already mapped. The survey is *everything on the server*, which is
            // the right answer to "what exists" and the wrong one to "what should I fetch": it
            // re-downloaded every header in the mailbox on every pass. On the maildrop this was
            // measured against that is 2372 `TOP` commands every five minutes, against a campus
            // server, for mail already on disk.
            //
            // A message whose header is stored but whose body is not is *not* wanted here;
            // `fetch_bodies` asks `unfetched` for those, which is the question it answers.
            let held: std::collections::HashSet<RemoteRef> =
                self.store.remote_refs(mailbox)?.into_iter().collect();
            wanted.retain(|(remote, _)| !held.contains(remote));
            // The survey is in server order, oldest first: POP3 message numbers and IMAP UIDs
            // both count up as mail arrives. Nothing else is known before the headers are.
            wanted.reverse();
        }
        by_band(&mut wanted);

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
                Ok(ProtoOutcome::Fetched { items, flags }) => {
                    let arrivals = items
                        .into_iter()
                        .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
                        .collect::<Vec<_>>();
                    report.headers_fetched += arrivals.len();
                    // Headers only: Body::Absent says the body has not arrived, rather than
                    // storing an empty message that looks complete.
                    let stored = crate::assemble::absorb_into(
                        &self.store,
                        self.account,
                        crate::assemble::Destination {
                            mailbox: mailbox.clone(),
                            role: self.role_of(mailbox),
                        },
                        // No cursor: this batch fetched a list it was handed and never asked
                        // the server what exists.
                        None,
                        arrivals,
                        true,
                        now,
                    )?;
                    report.arrived.extend(first_stored(&stored));
                    // The server's own view of these messages, which arrived on the same FETCH.
                    // Applied after absorbing, because a flag needs a message to sit on, and
                    // through `Ingest` because that is the path the sweep already uses. Without
                    // it every message is built unread and only a later sweep can correct it —
                    // which on a CONDSTORE server never revisits old mail, so anything found by
                    // backfill stayed unread for ever.
                    if !flags.is_empty() {
                        self.store.ingest(
                            self.account,
                            mail_domain::Ingest {
                                mailbox: mailbox.clone(),
                                validity: mail_domain::UidValidity::Same,
                                cursor: None,
                                messages: Vec::new(),
                                flags,
                                labels: Vec::new(),
                                label_names: Vec::new(),
                                gone: Vec::new(),
                            },
                        )?;
                    }
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
        by_band(&mut wanted);

        // A size of `u64::MAX` is "unknown", not "enormous": no survey this session. Those are
        // fetched whole, as everything was before large messages were fetched by part.
        let (parted, whole): (Vec<_>, Vec<_>) =
            wanted.into_iter().take(budget).partition(|(remote, size)| {
                matches!(remote, RemoteRef::Imap { .. })
                    && *size != u64::MAX
                    && *size > PARTED_ABOVE
            });
        let mut batch: Vec<RemoteRef> = whole.into_iter().map(|(remote, _)| remote).collect();
        if !parted.is_empty() {
            let parted = parted.into_iter().map(|(remote, _)| remote).collect();
            match self
                .fetch_parted(parted, mailbox, cancel, now, &mut report)
                .await
            {
                Ok(leftover) => batch.extend(leftover),
                Err(RuntimeError::Cancelled) => return Ok(report),
                Err(e) => return Err(e),
            }
        }
        if batch.is_empty() {
            return Ok(report);
        }
        match self
            .run(ProtoOp::FetchBody { remotes: batch }, cancel)
            .await
        {
            // Bodies carry flags too, but the header fetch has already recorded them and a
            // body batch is a subset; ignored rather than applied twice.
            Ok(ProtoOutcome::Fetched { items, .. }) => {
                let arrivals = items
                    .into_iter()
                    .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
                    .collect::<Vec<_>>();
                report.bodies_fetched += arrivals.len();
                crate::assemble::absorb_into(
                    &self.store,
                    self.account,
                    crate::assemble::Destination {
                        mailbox: mailbox.clone(),
                        role: self.role_of(mailbox),
                    },
                    None,
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

    /// Fetch large messages as their text and structure, leaving attachments on the server.
    ///
    /// Returns what could not be done that way, for the caller to fetch whole — which is always
    /// correct, only slower. That covers a server that fails to produce a structure (F112 is one
    /// that crashed doing it), a message that is not multipart and so has nothing to leave
    /// behind, and any section that did not come back.
    async fn fetch_parted(
        &mut self,
        remotes: Vec<RemoteRef>,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<Vec<RemoteRef>, RuntimeError> {
        let trees = match self
            .run(
                ProtoOp::FetchStructure {
                    remotes: remotes.clone(),
                },
                cancel,
            )
            .await
        {
            Ok(ProtoOutcome::Structures(trees)) => trees,
            Err(RuntimeError::Cancelled) => return Err(RuntimeError::Cancelled),
            Ok(_) | Err(_) => return Ok(remotes),
        };
        let mut whole: Vec<RemoteRef> = remotes
            .into_iter()
            .filter(|r| !trees.iter().any(|(described, _)| described == r))
            .collect();
        for (remote, tree) in trees {
            let Some(sections) = mail_mime::sections_for(&tree, &keep_now) else {
                whole.push(remote);
                continue;
            };
            let parts = match self
                .run(
                    ProtoOp::FetchSections {
                        remote: remote.clone(),
                        sections,
                    },
                    cancel,
                )
                .await
            {
                Ok(ProtoOutcome::Sections { parts, .. }) => parts,
                Err(RuntimeError::Cancelled) => return Err(RuntimeError::Cancelled),
                Ok(_) | Err(_) => {
                    whole.push(remote);
                    continue;
                }
            };
            let fetched: std::collections::HashMap<String, Vec<u8>> = parts.into_iter().collect();
            let Some(raw) = mail_mime::reconstruct(&tree, &fetched) else {
                whole.push(remote);
                continue;
            };
            crate::assemble::absorb_rebuilt_into(
                &self.store,
                self.account,
                crate::assemble::Destination {
                    mailbox: mailbox.clone(),
                    role: self.role_of(mailbox),
                },
                vec![crate::assemble::Arrival { remote, raw }],
                now,
            )?;
            report.bodies_fetched += 1;
        }
        Ok(whole)
    }

    /// Download one attachment a sync left on the server, and record it as held.
    ///
    /// The one place a part is fetched on its own: when the user opens or saves it. Its headers
    /// come with it, because they say how the bytes are encoded.
    pub async fn fetch_part(
        &mut self,
        message: MessageId,
        section: &str,
        cancel: &mut Cancel,
    ) -> Result<BlobId, RuntimeError> {
        let remote = self
            .store
            .remotes_of(message)?
            .into_iter()
            .find(|r| matches!(r, RemoteRef::Imap { .. }))
            .ok_or_else(|| {
                RuntimeError::Proto(mail_proto::ProtoError::Unsupported(
                    "fetching part of a message that has no IMAP address".to_owned(),
                ))
            })?;
        let header = format!("{section}.MIME");
        let outcome = self
            .run(
                ProtoOp::FetchSections {
                    remote,
                    sections: vec![header.clone(), section.to_owned()],
                },
                cancel,
            )
            .await?;
        let ProtoOutcome::Sections { parts, .. } = outcome else {
            return Err(RuntimeError::Proto(mail_proto::ProtoError::Malformed(
                "a section fetch answered with something else".to_owned(),
            )));
        };
        let find = |name: &str| {
            parts
                .iter()
                .find(|(s, _)| s == name)
                .map(|(_, bytes)| bytes.as_slice())
        };
        let (Some(mime), Some(content)) = (find(&header), find(section)) else {
            return Err(RuntimeError::Proto(mail_proto::ProtoError::Malformed(
                format!("the server did not send section {section} and its headers"),
            )));
        };
        let bytes = mail_mime::decode_part(mime, content);
        let blob = self
            .store
            .blobs()
            .put(&self.store.connection(), &bytes)
            .map_err(RuntimeError::Store)?;
        self.store
            .hold_part(message, section, blob, bytes.len() as u64)?;
        Ok(blob)
    }

    /// Which role a folder serves on this account.
    ///
    /// From the capabilities the last `LIST (SPECIAL-USE)` wrote down, falling back to `Inbox`:
    /// a server that answers no folder roles has one mailbox as far as this client is concerned,
    /// and POP3 has exactly one by construction. `INBOX` is matched case-insensitively because
    /// RFC 3501 says that name is, and a server that spells it `Inbox` is not describing a
    /// different mailbox.
    fn role_of(&self, mailbox: &MailboxRef) -> MailboxRole {
        if mailbox.path.eq_ignore_ascii_case("INBOX") {
            return MailboxRole::Inbox;
        }
        self.backend
            .caps()
            .folders
            .role(&mailbox.path)
            .unwrap_or(MailboxRole::Inbox)
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
            let since = self.resync_from(mailbox);
            match self
                .run(
                    ProtoOp::ListRemote {
                        mailbox: mailbox.clone(),
                        since,
                    },
                    cancel,
                )
                .await
            {
                Ok(ProtoOutcome::Resynced {
                    mut ingest,
                    vanished,
                }) => {
                    // The server named what vanished, so there is no listing to diff — and
                    // diffing anyway would find every message missing from a response that
                    // lists none. Only UIDs held here can be gone; the ranges are as wide as the
                    // server cared to make them.
                    ingest.gone = self
                        .store
                        .remote_refs(mailbox)?
                        .into_iter()
                        .filter(|held| match (held, since) {
                            (
                                RemoteRef::Imap {
                                    uidvalidity, uid, ..
                                },
                                Some(since),
                            ) => {
                                *uidvalidity == since.uidvalidity
                                    && vanished.iter().any(|&(lo, hi)| (lo..=hi).contains(uid))
                            }
                            _ => false,
                        })
                        .collect();
                    self.store.ingest(self.account, *ingest)?;
                    self.last.expunges = Some(now);
                }
                Ok(ProtoOutcome::Ingested(mut ingest)) => {
                    // The backend can only say what still exists. What we *hold* is the store's
                    // knowledge, so the diff happens here — and without it `gone` was always
                    // empty and a message deleted on another device never disappeared.
                    ingest.gone = self.vanished(mailbox)?;
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

    /// Remote addresses we hold that the server's listing no longer mentions.
    ///
    /// The server's answer is authoritative about existence and says nothing about identity, so
    /// this compares addresses, not messages. A message still present under another mailbox's
    /// UID keeps that row; only the mapping in *this* mailbox goes.
    ///
    /// An empty listing is a real answer — an emptied mailbox — and not a failure to parse, so
    /// it is allowed to expunge everything. That is the one case worth being sure about, since
    /// the alternative reading would delete a user's mail on a malformed response.
    fn vanished(&self, mailbox: &MailboxRef) -> Result<Vec<RemoteRef>, RuntimeError> {
        let still_there: std::collections::HashSet<RemoteRef> = self
            .backend
            .surveyed()
            .into_iter()
            .map(|(remote, _)| remote)
            .collect();
        Ok(self
            .store
            .remote_refs(mailbox)?
            .into_iter()
            .filter(|held| !still_there.contains(held))
            .collect())
    }

    /// The state to `QRESYNC` from, or `None` for a full listing.
    ///
    /// The same trust as [`Self::trusted_modseq`] — a modseq the server gave us and nobody has
    /// withdrawn — plus the server offering `QRESYNC`, and the UIDVALIDITY the modseq belongs to.
    fn resync_from(&self, mailbox: &MailboxRef) -> Option<Resync> {
        if self.backend.caps().condstore != Condstore::Qresync {
            return None;
        }
        let modseq = self.trusted_modseq(mailbox)?;
        match self.store.cursor(mailbox) {
            Ok(Some(SyncCursor::Imap { uidvalidity, .. })) => Some(Resync {
                uidvalidity,
                modseq,
            }),
            _ => None,
        }
    }

    /// A modseq worth fetching from, or `None` to fetch every flag.
    ///
    /// Three things must all hold, and any one of them failing means a full flag fetch:
    ///
    /// 1. The server advertises `CONDSTORE`. Sending `CHANGEDSINCE` to one that does not is a
    ///    protocol error, not a graceful degradation.
    /// 2. A cursor exists for this mailbox, carrying a `HIGHESTMODSEQ` the server gave us.
    /// 3. That modseq is non-zero. Zero is what this code records when the server said nothing,
    ///    and `CHANGEDSINCE 0` asks for everything anyway — more slowly, over a syntax the
    ///    server may reject.
    ///
    /// Deliberately *not* checking that the modseq advanced since last time. Dovecot 2.0.18
    /// froze `HIGHESTMODSEQ` at 1 while `EXISTS` climbed, and the defence against that belongs
    /// where it already is — withdrawing `CONDSTORE` from the account's capabilities — not in a
    /// second, quieter rule here that would make the two disagree.
    fn trusted_modseq(&self, mailbox: &MailboxRef) -> Option<u64> {
        if !self.backend.caps().condstore.changedsince() {
            return None;
        }
        match self.store.cursor(mailbox) {
            Ok(Some(SyncCursor::Imap { modseq, .. })) => modseq.filter(|m| *m > 0),
            // A POP cursor has no modseq, and a mailbox never synced has no cursor. Neither is
            // an error: both mean "fetch every flag", which is what `None` says.
            _ => None,
        }
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
        // Sizes from this session's survey. A message it did not cover — no survey yet, or a
        // protocol that cannot take one — is "size unknown" and sorts into the last band.
        let sizes: std::collections::HashMap<RemoteRef, u64> =
            self.backend.surveyed().into_iter().collect();
        Ok(self
            .store
            .unfetched(self.account, limit)?
            .into_iter()
            .map(|remote| {
                let size = sizes.get(&remote).copied().unwrap_or(u64::MAX);
                (remote, size)
            })
            .collect())
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

    fn pop(uidl: &str, size: u64) -> (RemoteRef, u64) {
        (
            RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            size,
        )
    }

    fn uidls(wanted: &[(RemoteRef, u64)]) -> Vec<&str> {
        wanted
            .iter()
            .map(|(remote, _)| match remote {
                RemoteRef::Pop { uidl } => uidl.as_str(),
                RemoteRef::Imap { .. } => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn a_band_keeps_the_order_it_was_given() {
        // Newest first in, newest first out, even where the newer message is the larger.
        let mut wanted = vec![pop("new", 4_000), pop("old", 3_000)];
        by_band(&mut wanted);
        assert_eq!(uidls(&wanted), ["new", "old"]);
    }

    #[test]
    fn a_smaller_band_comes_first_however_old() {
        let mut wanted = vec![
            pop("new-huge", 5 * 1024 * 1024),
            pop("new-medium", 200 * 1024),
            pop("old-small", 1_000),
            pop("unknown", u64::MAX),
        ];
        by_band(&mut wanted);
        assert_eq!(
            uidls(&wanted),
            ["old-small", "new-medium", "new-huge", "unknown"]
        );
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
