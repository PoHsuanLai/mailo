//! One account's worth of work: sync passes, and draining the outbox.
//!
//! One generic parameter, not five. `Store` and `Secrets` are process-wide singletons — one
//! SQLite file with one WAL connection, one keyring — so making them type parameters would
//! misrepresent ownership, and `Box<dyn Store>` per account would be worse. Time is an argument
//! rather than a `Clock` trait, per `CONVENTIONS.md` §6.

use crate::tokens::{AfterRefusal, Token, TokenSource};
use crate::{AccountSecrets, Cancel, RuntimeError, Transport, drive};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountPlan, BlobId, Condstore, FetchSince, Incoming, MailboxRef, MailboxRole,
    MessageId, Outgoing, PartTree, ProtoOp, RemoteRef, Resync, Retry, Retryable, SendState,
    SyncCursor, SystemFlag, Tls, UidValidity, WatchMode,
};
use mail_mime::Posting;
use mail_proto::backend::SmtpBackend;
use mail_proto::{Backend, Moved, ProtoOutcome, Submission};
use mail_store::{Dispatch, OutboxEntry, Settle, SqliteStore, Store};
use porter_core::{AccountId, Credential, Family, SecretKey, SecretPurpose, SecretText};
use std::sync::Arc;
use std::time::Duration;

mod partial;
mod search;
mod split;
pub(crate) mod wait;
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

/// How much of what a sync left on the server one pass fetches for an account kept offline.
///
/// Two bounds, because either alone lets a pass run long: a count, for a folder of many
/// middling attachments, and bytes, for a few enormous ones. The first part is always taken
/// whatever its size, or a part larger than the byte bound would never be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartBudget {
    pub parts: u32,
    pub bytes: u64,
}

impl PartBudget {
    /// The prefix of `wanted`, in its order, that fits.
    pub fn take(self, wanted: Vec<mail_store::RemotePart>) -> Vec<mail_store::RemotePart> {
        let mut spent: u64 = 0;
        let mut out = Vec::new();
        for part in wanted.into_iter().take(self.parts as usize) {
            if !out.is_empty() && spent.saturating_add(part.size) > self.bytes {
                break;
            }
            spent = spent.saturating_add(part.size);
            out.push(part);
        }
        out
    }
}

/// Bodies fetched per operation from Microsoft Graph, which asks for each one separately.
const GRAPH_BODIES: usize = 10;

fn by_band(wanted: &mut [(RemoteRef, u64)]) {
    wanted.sort_by_key(|(_, size)| BANDS.iter().position(|&band| *size <= band));
}

/// What a sync pass did, for the caller to log or show.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub headers_fetched: usize,
    pub bodies_fetched: usize,
    /// Attachments a sync had left on the server, fetched by this pass because the account is
    /// kept offline in full ([`AccountEngine::fetch_remote_parts`]).
    pub parts_fetched: usize,
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
    /// The desktop's account service no longer lets Mail use the account (`Retry::NeedsGrant`):
    /// stopped on as `needs_reauth` is, and asked of the person as allowing, not signing in.
    pub needs_grant: bool,
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
    /// Arrived messages a rule acted on, with the rules that did, in order.
    pub ruled: Vec<(MessageId, Vec<String>)>,
    /// Messages uploaded into a mailbox by this pass's outbox (`ProtoOp::Append`).
    pub appended: usize,
}

/// Refuse queued entry `id`, given up because a message it names was never found
/// ([`Dispatch::Lost`], FINDINGS F155): settled as [`Retry::Fatal`], so its undo puts back what
/// the server has, and said, so the person whose change it was is told it did not happen.
pub(crate) fn given_up(
    store: &dyn Store,
    id: mail_domain::OutboxId,
    reason: String,
    now: DateTime<Utc>,
    report: &mut SyncReport,
) -> Result<(), RuntimeError> {
    let retry = Retry::Fatal(reason.clone());
    report.saw(&retry);
    report.needs_attention.push(reason.clone());
    store.outbox_settle(id, Settle::Failed { reason, retry }, now)?;
    Ok(())
}

/// The messages a patch stored for the first time.
///
/// Only meaningful for the patch of a header fetch: that path upserts a message only when its
/// key is new, since a header carries no body to fill in. A body fetch's patch upserts messages
/// already held, and must not be read with this.
pub(crate) fn first_stored(patch: &mail_domain::Patch) -> impl Iterator<Item = MessageId> + '_ {
    patch.changes.iter().filter_map(|change| match change {
        mail_domain::Change::MessageUpsert(message) => Some(message.id),
        _ => None,
    })
}

impl SyncReport {
    /// Fold in the pass that followed this one, for a drain that queued work for itself.
    /// What is still queued is the later pass's word, since it saw the queue last.
    fn absorb(&mut self, later: SyncReport) {
        self.outbox_settled += later.outbox_settled;
        self.submitted += later.submitted;
        self.appended += later.appended;
        self.needs_attention.extend(later.needs_attention);
        self.still_queued = later.still_queued;
        self.needs_reauth |= later.needs_reauth;
        self.needs_grant |= later.needs_grant;
        self.hold = match (self.hold, later.hold) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    /// Fold one operation's verdict into the pass.
    ///
    /// The two facts a caller has to *act* on, gathered in one place so the several sites that
    /// see a `Retry` cannot disagree about which ones matter. `Retry::Now` and `Retry::Fatal`
    /// are decided within the pass — reconnect, or give up on that operation — and leave nothing
    /// for the caller to schedule.
    pub fn saw(&mut self, retry: &Retry) {
        match retry {
            Retry::NeedsReauth => self.needs_reauth = true,
            Retry::NeedsGrant => self.needs_grant = true,
            // The longest wait anyone asked for. Ordinary backoffs land here too, at a minute or
            // so, and are absorbed by the caller's own interval; a rate limit is an hour and is
            // not. Both mean the same thing to a scheduler, so neither needs a special case.
            Retry::After(wait) => self.hold = Some(self.hold.map_or(*wait, |had| had.max(*wait))),
            Retry::Now | Retry::Fatal(_) => {}
        }
    }
}

/// Who files the copy of a message an account submits; see `AccountEngine::sent_copy`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SentCopy {
    /// The server does, and a sync reads it back.
    Server,
    /// This client does, in the local Sent: the account has no mailbox for it.
    Here,
    /// This client uploads it to the server's Sent mailbox at this path.
    Upload(String),
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
    secrets: Arc<dyn AccountSecrets>,
    schedule: Schedule,
    last: LastRun,
    /// Where [`Outgoing::Graph`] posts. Graph's own address, except under test.
    graph_url: String,
    /// Keeps an OAuth account's tokens fresh between and within passes. `None` for a password
    /// account, and for an engine nobody gave one — which then signs in with what it was built
    /// with, as every engine did before a watch had to outlive an access token.
    tokens: Option<Arc<dyn TokenSource>>,
    /// Where an account that reads through Microsoft Graph sends its operations, in place of
    /// `backend` and a connection. `None` for every other account.
    graph: Option<crate::graph::read::Reader>,
    /// The mailboxes synced in full since the last drain: the folders a message the server moved
    /// without saying where has been looked for, counted when the drain begins
    /// ([`Store::unplaced_pass`], FINDINGS F155).
    synced: Vec<String>,
}

impl<B: Backend> AccountEngine<B> {
    pub fn new(
        account: AccountId,
        plan: AccountPlan,
        backend: B,
        store: Arc<SqliteStore>,
        secrets: Arc<dyn AccountSecrets>,
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
            tokens: None,
            graph: None,
            synced: Vec::new(),
        }
    }

    /// Read this account through Microsoft Graph: every operation goes to `reader`, over HTTPS,
    /// and none to the backend, which is then [`crate::graph::read::OverHttp`].
    pub fn with_graph_reader(mut self, reader: crate::graph::read::Reader) -> Self {
        self.graph = Some(reader);
        self
    }

    /// What the server is believed to support: the Graph reader's, where there is one.
    fn caps(&self) -> &AccountCaps {
        match &self.graph {
            Some(reader) => reader.caps(),
            None => self.backend.caps(),
        }
    }

    /// Every address the last survey found, with a size where one is known.
    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        match &self.graph {
            Some(reader) => reader.surveyed(),
            None => self.backend.surveyed(),
        }
    }

    /// Whether the incoming server can be reached, checked once before a pass.
    ///
    /// A connection opened and dropped, for a server that has one. Graph has no connection to
    /// open, and its first request answers the question soon enough.
    pub async fn reachable(&self) -> Result<(), RuntimeError> {
        if self.graph.is_some() {
            return Ok(());
        }
        drop(self.connect().await?);
        Ok(())
    }

    /// Renew the account's access tokens as they near expiry, and once after a refusal.
    ///
    /// The backend's session factory must read its credential from the source's `Held` cell, or
    /// the renewed token never reaches a connection.
    pub fn with_tokens(mut self, tokens: Arc<dyn TokenSource>) -> Self {
        self.tokens = Some(tokens);
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
            // No socket to open for Graph or JMAP: both are HTTPS. JMAP has an engine of its
            // own (`crate::jmap::JmapEngine`); an `AccountEngine` built for one connects nowhere.
            Incoming::Local | Incoming::Graph | Incoming::Jmap { .. } => None,
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
        let family = match self.plan.incoming {
            Incoming::Pop3 { .. } => Family::Pop3,
            _ => Family::Imap,
        };
        match self.relayed(family).await? {
            Some(relay) => Ok(relay),
            None => Transport::connect(host, port, tls).await,
        }
    }

    /// A connection to the account's server of `family` that accountd has already signed in to,
    /// when the account is accountd's ([`mail_domain::AuthPlan::Granted`]); `None` when it is mailo's own.
    ///
    /// An accountd account with no link to accountd (it is not running, or this is a build that
    /// has no D-Bus) cannot be reached, and says so as something to try again, not as a password
    /// to ask for.
    async fn relayed(&self, family: Family) -> Result<Option<Transport>, RuntimeError> {
        let Some(grant) = self.plan.grant() else {
            return Ok(None);
        };
        let Some(link) = self.secrets.link() else {
            return Err(RuntimeError::Link {
                why: format!(
                    "{} is an account of the desktop's account service, which is not reachable",
                    self.plan.address
                ),
                retry: Retry::After(Duration::from_secs(30)),
            });
        };
        let Some(endpoint) = self.plan.endpoint(family) else {
            return Err(RuntimeError::Link {
                why: format!(
                    "the grant on {} lists no {} server",
                    self.plan.address,
                    family.slug()
                ),
                retry: Retry::Fatal(format!("no {} server granted", family.slug())),
            });
        };
        Ok(Some(link.open(grant, endpoint).await?))
    }

    /// Run one operation, with a token that is fresh — and, if the server refuses a token that
    /// looked valid, once more with a new one.
    ///
    /// Here rather than in each operation, because every one of them presents a token and a watch
    /// reaches all of them an hour in. A token can be refused before its expiry says it should
    /// be: revoked sessions, a clock that is wrong, an issuer that shortened a lifetime. One
    /// renewal answers that; a second would not, which is why [`TokenSource::after_refusal`] will not
    /// renew a token it minted for this reason already.
    async fn run(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        self.run_leaving(op, cancel, None).await
    }

    /// [`Self::run_leaving`], once per IMAP mailbox the operation's messages are in
    /// ([`split::per_mailbox`], FINDINGS F154).
    ///
    /// Where a split one is interrupted, what already went out stays out and is recorded: a
    /// move's new addresses are kept as each part answers, so the retry of the whole, addressed
    /// again when it is sent, finds those messages where they now are, and a flag set twice is
    /// the same flag. `progress` is left saying which parts the server answered and which it did
    /// not, so a refusal undoes only the rest (FINDINGS F159); it stays empty for one part.
    async fn run_per_mailbox(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
        leaving: Option<DateTime<Utc>>,
        progress: &mut partial::Progress,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let mut parts = split::per_mailbox(op);
        if parts.len() == 1 {
            let op = parts.pop().expect("one part");
            return self.run_leaving(op, cancel, leaving).await;
        }
        let mut parts = parts.into_iter();
        while let Some(part) = parts.next() {
            let into = mail_proto::backend::imap::move_target(self.caps(), &part);
            match self.run_leaving(part.clone(), cancel, leaving).await {
                Ok(outcome) => {
                    // Answered, whatever recording where it went says next.
                    progress.answered.push(part);
                    if let ProtoOutcome::Moved(moved) = outcome {
                        self.moved(&moved, into.as_deref())?;
                    }
                }
                Err(e) => {
                    progress.left.push(part);
                    progress.left.extend(parts);
                    return Err(e);
                }
            }
        }
        Ok(ProtoOutcome::Applied)
    }

    /// [`Self::run`], with the moment a submission leaves: its `Date` is set to `leaving` on
    /// the way out. `None` for everything that is not a submission.
    async fn run_leaving(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
        leaving: Option<DateTime<Utc>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let Some(tokens) = &self.tokens else {
            return self.run_once(op, cancel, leaving).await;
        };
        let token = match op {
            ProtoOp::Submit { .. } => Token::Sending,
            _ => Token::Incoming,
        };
        tokens.ahead(token).await?;
        let first = self.run_once(op.clone(), cancel, leaving).await;
        let refused = matches!(&first, Err(e) if matches!(e.retry(), Retry::NeedsReauth));
        let Some(tokens) = self.tokens.as_ref().filter(|_| refused) else {
            return first;
        };
        match tokens.after_refusal(token).await? {
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
        if self.graph.is_some() {
            return self.run_graph(op, cancel).await;
        }
        // An upload's bytes are a blob, and reading one is this crate's to do. Staged here, per
        // attempt, rather than by the caller: a retry after a renewed sign-in runs this again,
        // and bytes staged once would already have been spent by the first attempt.
        if let ProtoOp::Append { raw, .. } = &op {
            let bytes = self.store.blobs().get(*raw)?;
            self.backend.stage_append(bytes);
        }
        let mut transport = self.connect().await?;
        let mut step = BackendMachine {
            backend: &mut self.backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// Run one operation through Microsoft Graph, and remap whatever it moved.
    ///
    /// The token is Graph's, kept as the account's outgoing credential: an account that reads
    /// through Graph presents the same one it sends with. Remapped here, per attempt, so a move
    /// made before a later failure in the same batch is not forgotten.
    async fn run_graph(
        &mut self,
        op: ProtoOp,
        cancel: &mut Cancel,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let access = match self.presented(Token::Sending).await {
            Ok(Credential::OAuth { access, .. }) => access,
            _ => {
                return Err(RuntimeError::Secrets(format!(
                    "no Microsoft Graph sign-in is stored for {}",
                    self.plan.address
                )));
            }
        };
        let staged = match &op {
            ProtoOp::Append { raw, .. } => {
                Some(self.store.blobs().get(*raw)?)
            }
            _ => None,
        };
        let Some(reader) = self.graph.as_mut() else {
            return Err(RuntimeError::UnsupportedIo(
                "this account does not read through Microsoft Graph".to_owned(),
            ));
        };
        let outcome = tokio::select! {
            outcome = reader.run(op, access.expose(), staged) => outcome,
            () = cancelled(cancel) => Err(RuntimeError::Cancelled),
        };
        let moves = reader.take_moves();
        for (from, to) in moves {
            self.store.remap(self.account.clone(), &from, &to)?;
        }
        outcome
    }

    /// Where the submission server lives, for an account that submits over SMTP.
    fn outgoing(&self) -> Option<(&str, u16, Tls)> {
        match &self.plan.outgoing {
            Outgoing::Smtp { host, port, tls } => Some((host, *port, *tls)),
            Outgoing::Graph | Outgoing::Jmap | Outgoing::Nowhere => None,
        }
    }

    /// A backend that can submit exactly one message for this account.
    ///
    /// Built per submission rather than held, because it closes over the credential and a
    /// long-lived copy of a password is a copy waiting to be logged. The closure is the only
    /// thing that holds it; `SmtpBackend` itself never sees it.
    async fn submitter(&self) -> Result<SmtpBackend, RuntimeError> {
        let (host, port, tls) = self.outgoing().ok_or_else(|| {
            RuntimeError::UnsupportedIo("this account does not submit over SMTP".to_owned())
        })?;
        let (host, ehlo) = (host.to_owned(), self.plan.ehlo());
        let (username, sasl) = (self.plan.username(), self.plan.sasl());
        // Most providers authenticate submission with the same secret as retrieval, which is
        // what `AuthPlan` means by covering both directions. A separate outgoing secret is
        // preferred where one was stored, because a few hosts really do differ.
        let relayed = self.plan.grant().is_some();
        let credential = if relayed {
            // The relay signs in: nothing to present, and nothing of the account's in this process.
            Credential::Password(SecretText::new(""))
        } else {
            match &self.tokens {
                Some(tokens) => tokens.current(Token::Sending).await?,
                None => match self.secret(SecretPurpose::OutgoingPassword).await {
                    Ok(credential) => credential,
                    Err(_) => self.secret(SecretPurpose::IncomingPassword).await?,
                },
            }
        };
        // The incoming backend's capabilities. They describe the *account*, not the socket:
        // `SmtpBackend` reads none of the IMAP-shaped fields, and giving it a second, emptier
        // set would mean two answers to one question.
        let caps = self.caps().clone();
        Ok(SmtpBackend::new(
            self.account.clone(),
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
                    relayed,
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

    /// The credential to present for `token`: the token source's when the account has one, which
    /// is the only place an OAuth token comes from, else what the account's secrets hold.
    async fn presented(&self, token: Token) -> Result<Credential, RuntimeError> {
        match &self.tokens {
            Some(tokens) => tokens.current(token).await,
            None => {
                self.secret(match token {
                    Token::Sending => SecretPurpose::OutgoingPassword,
                    Token::Incoming => SecretPurpose::IncomingPassword,
                })
                .await
            }
        }
    }

    async fn secret(&self, purpose: SecretPurpose) -> Result<Credential, RuntimeError> {
        self.secrets
            .get(&SecretKey {
                account: self.account.clone(),
                purpose,
            })
            .await
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
        // Submitted by `crate::jmap::JmapEngine`, which holds the session this needs. Handing it
        // to Graph below — the fallthrough for "not SMTP" — would post it to the wrong company.
        if self.plan.outgoing == Outgoing::Jmap {
            return Err(RuntimeError::UnsupportedIo(
                "a JMAP account sends through its JMAP engine".to_owned(),
            ));
        }
        let message = self.leaving_bytes(raw, leaving)?;
        let Some((host, port, tls)) = self.outgoing() else {
            return self.submit_to_graph(&message, rcpt_to).await;
        };
        let (host, port, tls) = (host.to_owned(), port, tls);
        let posting = Posting {
            mail_from: mail_from.clone(),
            rcpt_to: rcpt_to.clone(),
            message,
        };

        let mut backend = self.submitter().await?;
        backend.stage(posting)?;
        let mut transport = match self.relayed(Family::Smtp).await? {
            Some(relay) => relay,
            None => Transport::connect(&host, port, tls).await?,
        };
        let mut step = BackendMachine {
            backend: &mut backend,
            op: Some(op),
        };
        drive(&mut step, &mut transport, cancel).await
    }

    /// The bytes a submission hands the server: frozen when the user pressed send, so a draft
    /// edited while the outbox was backed off does not change what goes out, and dated `leaving`.
    /// One place, so the copy [`Self::keep_sent`] keeps is the message that went.
    fn leaving_bytes(
        &self,
        raw: BlobId,
        leaving: Option<DateTime<Utc>>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let frozen = self.store.blobs().get(raw)?;
        Ok(match leaving {
            Some(at) => mail_mime::restamp(&frozen, at),
            None => frozen,
        })
    }

    /// Who files the copy of a message this account submits.
    ///
    /// SMTP says only that a message was accepted (RFC 6409), so whether a Sent copy exists
    /// afterwards depends on the pairing of servers, and each has one answer:
    ///
    /// - Graph's `sendMail`, JMAP's `EmailSubmission` and a Gmail or Microsoft SMTP server file
    ///   it themselves, and a sync reads it back from Sent. Keeping one here too would list the
    ///   message twice.
    /// - An IMAP account on any other SMTP server has a Sent mailbox and nobody filing into it,
    ///   so the client uploads the copy there (`APPEND`, RFC 3501 §6.3.11) when the server's
    ///   `SPECIAL-USE` listing names one (RFC 6154). With none named, uploading to a guessed
    ///   path is how a message lands somewhere nobody looks, so the copy is kept here.
    /// - A POP3 or local account has no Sent mailbox at all, whichever server sends for it, so
    ///   the copy is kept here: Graph's lands in a Sent Items a POP3 sync never reads.
    fn sent_copy(&self) -> SentCopy {
        match (&self.plan.incoming, &self.plan.outgoing) {
            (_, Outgoing::Jmap | Outgoing::Nowhere)
            | (Incoming::Jmap { .. } | Incoming::Graph, _) => SentCopy::Server,
            (Incoming::Pop3 { .. } | Incoming::Local, _) => SentCopy::Here,
            (Incoming::Imap { .. }, Outgoing::Graph) => SentCopy::Server,
            (Incoming::Imap { .. }, Outgoing::Smtp { host, .. }) => {
                if mail_domain::presets::files_sent_itself(host) {
                    SentCopy::Server
                } else {
                    match self.folder_for(MailboxRole::Sent) {
                        Some(path) => SentCopy::Upload(path),
                        None => SentCopy::Here,
                    }
                }
            }
        }
    }

    /// Keep what was just submitted as `raw`, dated `at`, as [`Self::sent_copy`] says: kept
    /// here in Sent (see [`crate::assemble::sent`]), or queued as an upload into the server's
    /// Sent mailbox so that going offline or a refusal retries it like any other operation.
    ///
    /// The copy is the message that went, with the blind recipients named in a `Bcc` field
    /// (RFC 5322 §3.6.3): the transmitted bytes never had one, and the sender's copy should.
    /// The stored message when kept here; `None` for an upload, which a sync finds in Sent.
    fn keep_sent(
        &self,
        how: &SentCopy,
        raw: BlobId,
        rcpt_to: &[String],
        at: DateTime<Utc>,
    ) -> Result<Option<MessageId>, RuntimeError> {
        let sent = self.leaving_bytes(raw, Some(at))?;
        let copy = mail_mime::with_blind(&sent, rcpt_to);
        match how {
            SentCopy::Server => Ok(None),
            SentCopy::Here => crate::assemble::sent(&self.store, self.account.clone(), copy, at),
            SentCopy::Upload(path) => {
                let raw = self.store.blobs().put(&self.store.connection(), &copy)?;
                self.store.enqueue(
                    self.account.clone(),
                    mail_domain::RemoteIntent::Append {
                        mailbox: MailboxRef {
                            account: self.account.clone(),
                            path: path.clone(),
                        },
                        flags: vec![SystemFlag::Seen],
                        date: Some(at),
                        raw,
                    },
                    // Nothing to undo: an upload changes nothing here until it has happened.
                    &mail_domain::Patch {
                        id: mail_domain::ChangeId::generate(),
                        changes: Vec::new(),
                    },
                    at,
                )?;
                Ok(None)
            }
        }
    }

    /// Submit through Microsoft Graph, for an account whose plan says [`Outgoing::Graph`].
    ///
    /// The token is the account's *outgoing* credential: a Graph access token, which the caller
    /// minted from the sign-in's refresh token before the pass (`tokens::graph_token`). The
    /// incoming one is for Exchange's IMAP and Graph refuses it. Graph files the sent copy
    /// itself, and does not say where.
    async fn submit_to_graph(
        &self,
        message: &[u8],
        rcpt_to: &[String],
    ) -> Result<ProtoOutcome, RuntimeError> {
        let credential = self.presented(Token::Sending).await?;
        let porter_core::Credential::OAuth { access, .. } = credential else {
            return Err(RuntimeError::Secrets(
                "sending through Graph needs a Microsoft sign-in, not a password".to_owned(),
            ));
        };
        let http = crate::http::http_client()?;
        crate::graph::send_mime(&http, &self.graph_url, access.expose(), message, rcpt_to).await?;
        Ok(ProtoOutcome::Submitted { remote: None })
    }

    /// Drain the outbox, in insertion order, stopping at the first entry not yet due.
    ///
    /// Insertion order is load-bearing: two operations on one thread must reach the server in
    /// the order the user performed them, or the result is whichever won the race.
    ///
    /// Each entry is addressed as it is reached ([`Store::outbox_dispatch`]), after every move
    /// before it in this drain has told the store where its messages went. One whose message
    /// has no address is left queued, untried, and goes in a later pass: a pass syncs before it
    /// drains, and that sync is what finds a message a server moved without saying where.
    ///
    /// Not for ever. A drain after syncs first counts them against every message still waiting
    /// to be found ([`Store::unplaced_pass`]); once the syncs that should have found one have
    /// not, an operation on it is refused, its local change undone, and the reason said
    /// (FINDINGS F155). A drain with no sync before it, such as `mailo`'s upload, counts nothing.
    pub async fn drain_outbox(
        &mut self,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<SyncReport, RuntimeError> {
        let (mut report, mut again) = self.drain_pass(cancel, now).await?;
        // A copy queued by a submission goes in the same call, unless something failed: the Sent
        // copy of a message is not left waiting for the next pass.
        while again {
            let (later, more) = self.drain_pass(cancel, now).await?;
            report.absorb(later);
            again = more;
        }
        Ok(report)
    }

    /// One walk through what is due. The flag says a submission in it queued an upload of its
    /// sent copy and nothing stopped the walk, so another is worth making at once.
    async fn drain_pass(
        &mut self,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<(SyncReport, bool), RuntimeError> {
        let mut report = SyncReport::default();
        let mut copy_queued = false;
        let mut halted = false;
        let synced = std::mem::take(&mut self.synced);
        if !synced.is_empty() {
            self.store.unplaced_pass(self.account.clone(), &synced)?;
        }
        for entry in self.store.outbox_due(self.account.clone(), now)? {
            let id = entry.id;
            let Some(entry) = self.addressed(entry, now, &mut report)? else {
                continue;
            };
            // Where a move files its messages, for a server that does not say where they went.
            let into = mail_proto::backend::imap::move_target(self.caps(), &entry.op);
            // Noted before the op is consumed. A submission also has a draft whose visible
            // state must follow what happened on the wire; nothing else in the outbox does.
            let draft = match &entry.op {
                ProtoOp::Submit { draft, .. } => Some(*draft),
                _ => None,
            };
            // What a submission sent, and to whom, for the account that files its own copy of it.
            let copy = self.sent_copy();
            let submitted = match &entry.op {
                ProtoOp::Submit { raw, rcpt_to, .. } if copy != SentCopy::Server => {
                    Some((*raw, rcpt_to.clone()))
                }
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
            // Whose each address is before anything goes, for an operation sent in parts: a
            // refusal after a part that moved its messages must still tell whose they were.
            let held = if split::per_mailbox(entry.op.clone()).len() > 1 {
                partial::held(&*self.store, &entry.undo)?
            } else {
                Vec::new()
            };
            let mut progress = partial::Progress::default();
            // Stamped with the same instant the draft records as `Sent { at }`, so the two agree.
            match self
                .run_per_mailbox(entry.op, cancel, draft.map(|_| now), &mut progress)
                .await
            {
                Ok(outcome) => {
                    // Before the settle, so the next entry, addressed as it is reached, finds
                    // the message where the move put it.
                    if let ProtoOutcome::Moved(moved) = &outcome {
                        self.moved(moved, into.as_deref())?;
                    }
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
                        // SMTP reports that the message was accepted, not where a copy was
                        // filed. Gmail files it in Sent itself and a sync finds it there, so
                        // `message: None` until then, and so for a copy this client uploads.
                        // A POP3 account has no Sent folder at all: the copy is kept here,
                        // now, or nowhere.
                        let message = match submitted {
                            Some((raw, rcpt_to)) => match self.keep_sent(&copy, raw, &rcpt_to, now)
                            {
                                Ok(kept) => kept,
                                // Sent whatever this says: failing the drain over the copy
                                // would report as unsent a message the server accepted.
                                Err(e) => {
                                    report
                                        .needs_attention
                                        .push(format!("sent, but no copy kept in Sent: {e}"));
                                    None
                                }
                            },
                            None => None,
                        };
                        copy_queued |= matches!(copy, SentCopy::Upload(_));
                        self.mark_draft(draft, SendState::Sent { at: now, message }, now);
                        report.submitted += 1;
                    }
                    report.outbox_settled += 1;
                }
                Err(RuntimeError::Cancelled) => return Ok((report, false)),
                Err(e) => {
                    let retry = e.retry();
                    report.saw(&retry);
                    // Refused after part of it was done: only the rest is undone (FINDINGS F159).
                    let done = match retry {
                        Retry::Fatal(_) => partial::done(&held, &progress, into.as_deref()),
                        _ => Vec::new(),
                    };
                    if !done.is_empty() {
                        report
                            .needs_attention
                            .push(partial::said(&progress, into.as_deref(), &e));
                        self.store.outbox_settle(id, Settle::InPart { done }, now)?;
                        halted = true;
                        break;
                    }
                    if retry.needs_person() || matches!(retry, Retry::Fatal(_)) {
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
                    halted = true;
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
                self.account.clone(),
                now + chrono::TimeDelta::try_days(365).unwrap_or_default(),
            )
            .map(|due| due.len())
            .unwrap_or(0);
        Ok((report, copy_queued && !halted))
    }

    /// `entry` pointed at where its messages are now, or `None` when there is nothing to send it
    /// yet: it is waiting for a sync to find a message, and stays queued as it is, or there was
    /// nothing left for it to say and it has been settled, or it has waited through the syncs
    /// that should have found its message and has been refused, and `report` says why.
    fn addressed(
        &self,
        entry: OutboxEntry,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<Option<OutboxEntry>, RuntimeError> {
        match self.store.outbox_dispatch(entry.id)? {
            Dispatch::Send(op) => Ok(Some(OutboxEntry { op, ..entry })),
            Dispatch::Wait => Ok(None),
            Dispatch::Moot => {
                self.store.outbox_settle(entry.id, Settle::Ok, now)?;
                Ok(None)
            }
            Dispatch::Lost(reason) => {
                // A move leaves alone what is already where it was taking it (FINDINGS F159).
                let target = mail_proto::backend::imap::move_target(self.caps(), &entry.op);
                let done = match &target {
                    Some(folder) => partial::already_there(
                        &*self.store,
                        self.account.clone(),
                        &entry.undo,
                        folder,
                    )?,
                    None => Vec::new(),
                };
                match target.filter(|_| !done.is_empty()) {
                    Some(folder) => {
                        let retry = Retry::Fatal(reason.clone());
                        report.saw(&retry);
                        report.needs_attention.push(format!(
                            "{reason} Of its messages, {} already in {folder} are left there.",
                            done.len()
                        ));
                        self.store
                            .outbox_settle(entry.id, Settle::InPart { done }, now)?;
                    }
                    None => given_up(&*self.store, entry.id, reason, now, report)?,
                }
                Ok(None)
            }
        }
    }

    /// Record where a move put each message: its new address where the server said, and none
    /// where it did not, so what follows it waits for the sync rather than go to the old one.
    /// `into` is the folder it was moved into, whose syncs are the ones to find it.
    fn moved(&self, moved: &[Moved], into: Option<&str>) -> Result<(), RuntimeError> {
        for one in moved {
            match &one.to {
                Some(to) => self.store.remap(self.account.clone(), &one.from, to)?,
                None => self.store.unmap(self.account.clone(), &one.from, into)?,
            }
        }
        Ok(())
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
        let bytes = self.store.blobs().get(raw)?;
        let role = self.role_of(&mailbox);
        match remote {
            Some(remote) => {
                crate::assemble::appended(
                    &self.store,
                    self.account.clone(),
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
                    self.account.clone(),
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
            self.store.put_caps(self.account.clone(), &caps, now)?;
        }
        // Folder roles, where the protocol has folders at all. A backend without them answers
        // something other than `Caps`, and this falls back to what the backend already believes
        // rather than treating it as a failure.
        let outcome = self.run(ProtoOp::ListFolders, cancel).await?;
        let caps = match outcome {
            ProtoOutcome::Folders { caps, listed } => {
                self.store.put_folders(self.account.clone(), listed)?;
                *caps
            }
            ProtoOutcome::Caps(caps) => *caps,
            _ => self.caps().clone(),
        };
        let caps = match self.graph.as_mut() {
            Some(reader) => reader.observed(now),
            None => caps,
        };
        self.store.put_caps(self.account.clone(), &caps, now)?;
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
            self.store.put_folders(self.account.clone(), listed)?;
            let caps = match self.graph.as_mut() {
                Some(reader) => reader.observed(now),
                None => *caps,
            };
            self.store.put_caps(self.account.clone(), &caps, now)?;
        }
        Ok(self.store.folders(self.account.clone())?)
    }

    /// Whether this account has folders and none have been listed yet.
    ///
    /// Capabilities are re-read daily, and the folder list with them. An account added this
    /// morning would otherwise show no folders until tomorrow, and refuse to rename or delete
    /// any, so a sync pass asks as soon as it finds the list empty.
    pub fn folders_unlisted(&self) -> bool {
        matches!(self.plan.incoming, Incoming::Imap { .. } | Incoming::Graph)
            && self
                .store
                .folders(self.account.clone())
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
        let raw = self.store.blobs().put(&raw)?;
        let op = ProtoOp::Append {
            mailbox: MailboxRef {
                account: self.account.clone(),
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
        if !matches!(self.caps().watch, WatchMode::Idle) {
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
        now.signed_duration_since(self.caps().observed_at)
            > chrono::TimeDelta::try_hours(24).expect("24h is in range")
    }

    /// Run the account's rules over what a pass stored for the first time.
    ///
    /// `arrived` is [`SyncReport::arrived`], so each message meets the rules once, when it first
    /// appears. Their actions are applied here and queued for the server with what the server
    /// was last seen to support; the outbox drain that ends the pass sends them.
    pub fn run_rules(
        &self,
        arrived: &[MessageId],
        now: DateTime<Utc>,
    ) -> Result<mail_store::rules::Ran, RuntimeError> {
        Ok(mail_store::rules::at_arrival(
            self.store.as_ref(),
            self.account.clone(),
            self.caps(),
            arrived,
            now,
        )?)
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
        if self.graph.is_some() {
            return self.sync_graph(mailbox, cancel, now, budget).await;
        }
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
            self.store.ingest(self.account.clone(), *ingest)?;
        }

        // What to fetch comes from the SERVER'S listing, not from the store. On a first sync
        // the store knows nothing, so asking it what is missing returns an empty list and the
        // pass fetches nothing — silently, with no error. Only an end-to-end test caught that.
        //
        // The ordering decision lives here rather than in the backend, because it is a product
        // judgement about what the user sees first, not a protocol fact.
        let mut wanted = self.surveyed();
        if wanted.is_empty() {
            // A protocol that cannot enumerate up front: fall back to what we already hold,
            // which the store hands back newest first.
            let first;
            (wanted, first) = self.unfetched(mailbox, budget as u32)?;
            by_band(&mut wanted[first..]);
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
            by_band(&mut wanted);
        }

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
                    report.headers_fetched += items.len();
                    let stored = self.absorb_headers(mailbox, items, flags, now)?;
                    report.arrived.extend(first_stored(&stored));
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
        // Every message the server listed is held now, so a message it moved here without
        // saying where would have been found by this sync. One the budget stopped short of is
        // not counted: the message may be among the rest.
        if wanted.is_empty() {
            self.synced.push(mailbox.path.clone());
        }
        Ok(report)
    }

    /// Keep the headers a `FetchHeaders` brought from `mailbox`, and the flags the server sent
    /// with them. The patch's upserts are the messages new to the account ([`first_stored`]).
    ///
    /// The one way headers are kept, whether a sync or a search of the server fetched them.
    pub(crate) fn absorb_headers(
        &self,
        mailbox: &MailboxRef,
        items: Vec<(RemoteRef, Vec<u8>)>,
        flags: Vec<(RemoteRef, mail_domain::ReadState, mail_domain::Star)>,
        now: DateTime<Utc>,
    ) -> Result<mail_domain::Patch, RuntimeError> {
        let arrivals = items
            .into_iter()
            .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
            .collect::<Vec<_>>();
        // Headers only: Body::Absent says the body has not arrived, rather than storing an empty
        // message that looks complete.
        let stored = crate::assemble::absorb_into(
            &self.store,
            self.account.clone(),
            crate::assemble::Destination {
                mailbox: mailbox.clone(),
                role: self.role_of(mailbox),
            },
            // No cursor: this batch fetched a list it was handed and never asked the server what
            // exists.
            None,
            arrivals,
            true,
            now,
        )?;
        // The server's own view of these messages, which arrived on the same FETCH. Applied
        // after absorbing, because a flag needs a message to sit on, and through `Ingest`
        // because that is the path the sweep already uses. Without it every message is built
        // unread and only a later sweep can correct it — which on a CONDSTORE server never
        // revisits old mail, so anything found by backfill stayed unread for ever.
        if !flags.is_empty() {
            self.store.ingest(
                self.account.clone(),
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
        Ok(stored)
    }

    /// [`Self::sync`] for an account read through Microsoft Graph: one folder's delta.
    ///
    /// The delta is the survey, the header fetch and the flag sweep at once. What it says is
    /// stored in the order that keeps a crash harmless: a reset first, where Graph asked for one;
    /// then the messages new to this folder, from the headers the delta wrote; then flags,
    /// removals and the cursor, last, so the place a later pass resumes from is never ahead of
    /// what is stored.
    async fn sync_graph(
        &mut self,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        budget: usize,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        let since = match self.store.cursor(mailbox)? {
            Some(cursor @ SyncCursor::Graph { .. }) => FetchSince::After { cursor },
            _ => FetchSince::Beginning,
        };
        if let Some(reader) = self.graph.as_mut() {
            reader.limit(budget);
        }
        let outcome = self
            .run(
                ProtoOp::FetchEnvelopes {
                    mailbox: mailbox.clone(),
                    since,
                },
                cancel,
            )
            .await?;
        let ProtoOutcome::Ingested(ingest) = outcome else {
            return Ok(report);
        };
        let mut ingest = *ingest;
        let arrivals = self
            .graph
            .as_mut()
            .map(|reader| reader.take_arrivals())
            .unwrap_or_default();

        if ingest.validity == UidValidity::Reset {
            self.store.ingest(
                self.account.clone(),
                mail_domain::Ingest {
                    mailbox: mailbox.clone(),
                    validity: UidValidity::Reset,
                    cursor: None,
                    messages: Vec::new(),
                    flags: Vec::new(),
                    labels: Vec::new(),
                    label_names: Vec::new(),
                    gone: Vec::new(),
                },
            )?;
            ingest.validity = UidValidity::Same;
        }

        // A message already mapped here arrived again because something about it changed: its
        // flags, which the ingest below applies. Only the rest are new to this folder.
        let held: std::collections::HashSet<RemoteRef> =
            self.store.remote_refs(mailbox)?.into_iter().collect();
        let new: Vec<crate::assemble::Arrival> = arrivals
            .into_iter()
            .filter(|(remote, _)| !held.contains(remote))
            .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
            .collect();
        if !new.is_empty() {
            report.headers_fetched += new.len();
            let stored = crate::assemble::absorb_into(
                &self.store,
                self.account.clone(),
                crate::assemble::Destination {
                    mailbox: mailbox.clone(),
                    role: self.role_of(mailbox),
                },
                None,
                new,
                true,
                now,
            )?;
            report.arrived.extend(first_stored(&stored));
        }
        self.store.ingest(self.account.clone(), ingest)?;
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
        let (mut wanted, first) = self.unfetched(mailbox, budget as u32)?;
        by_band(&mut wanted[first..]);
        wanted.truncate(budget);
        // What the window is showing, all of it, before the rest: within one group a large
        // message is fetched by part ahead of the small ones, and a large one from the backlog
        // must not keep a small one the window waits on waiting.
        let rest = wanted.split_off(first.min(wanted.len()));
        for group in [wanted, rest] {
            if !group.is_empty()
                && !self
                    .fetch_group(group, mailbox, cancel, now, &mut report)
                    .await?
            {
                break;
            }
        }
        Ok(report)
    }

    /// Fetch the bodies of `wanted`: the large ones by part, then the rest whole. Whether to go
    /// on with the next group.
    async fn fetch_group(
        &mut self,
        wanted: Vec<(RemoteRef, u64)>,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<bool, RuntimeError> {
        // A size of `u64::MAX` is "unknown", not "enormous": no survey this session. Those are
        // fetched whole, as everything was before large messages were fetched by part.
        let (parted, whole): (Vec<_>, Vec<_>) = wanted.into_iter().partition(|(remote, size)| {
            matches!(remote, RemoteRef::Imap { .. }) && *size != u64::MAX && *size > PARTED_ABOVE
        });
        let mut batch: Vec<RemoteRef> = whole.into_iter().map(|(remote, _)| remote).collect();
        if !parted.is_empty() {
            let parted = parted.into_iter().map(|(remote, _)| remote).collect();
            match self
                .fetch_parted(parted, mailbox, cancel, now, report)
                .await
            {
                Ok(leftover) => batch.extend(leftover),
                Err(RuntimeError::Cancelled) => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        if batch.is_empty() {
            return Ok(true);
        }
        // Graph fetches one body per request, so a batch is fetched a few at a time and a failure
        // loses only the few in hand; a session fetches its whole batch in one command.
        if self.graph.is_some() {
            for chunk in batch.chunks(GRAPH_BODIES) {
                if !self
                    .fetch_body_batch(chunk.to_vec(), mailbox, cancel, now, report)
                    .await?
                {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        self.fetch_body_batch(batch, mailbox, cancel, now, report)
            .await
    }

    /// Fetch one batch of whole bodies and store them. Whether to go on with the next.
    async fn fetch_body_batch(
        &mut self,
        batch: Vec<RemoteRef>,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<bool, RuntimeError> {
        match self
            .run(ProtoOp::FetchBody { remotes: batch }, cancel)
            .await
        {
            // Bodies carry flags too, but the header fetch has already recorded them and a
            // body batch is a subset; ignored rather than applied twice.
            Ok(ProtoOutcome::Fetched { items, .. }) => {
                report.bodies_fetched += self.store_bodies(items, mailbox, now)?;
            }
            Ok(_) => {}
            Err(RuntimeError::Cancelled) => return Ok(false),
            Err(e) => {
                report.saw(&e.retry());
                report.needs_attention.push(e.to_string());
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Store whole messages fetched from `mailbox`. How many.
    fn store_bodies(
        &self,
        items: Vec<(RemoteRef, Vec<u8>)>,
        mailbox: &MailboxRef,
        now: DateTime<Utc>,
    ) -> Result<usize, RuntimeError> {
        let arrivals = items
            .into_iter()
            .map(|(remote, raw)| crate::assemble::Arrival { remote, raw })
            .collect::<Vec<_>>();
        let count = arrivals.len();
        crate::assemble::absorb_into(
            &self.store,
            self.account.clone(),
            crate::assemble::Destination {
                mailbox: mailbox.clone(),
                role: self.role_of(mailbox),
            },
            None,
            arrivals,
            false,
            now,
        )?;
        Ok(count)
    }

    /// Fetch one message's body now, whole, and store it.
    ///
    /// What the reader asks for when it opens a message a sync has not yet reached. Unlike
    /// [`AccountEngine::fetch_bodies`] a failure is returned, not folded into a report: someone
    /// is waiting on this one message and has to be told why it did not come. Over IMAP and
    /// Graph, which address a message by its own reference; POP3 numbers messages within a
    /// survey that a single fetch has not taken.
    pub async fn fetch_body(
        &mut self,
        message: MessageId,
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<(), RuntimeError> {
        let graph = self.graph.is_some();
        let (remote, path) = self
            .store
            .remotes_of(message)?
            .into_iter()
            .find_map(|remote| match &remote {
                RemoteRef::Graph { mailbox, .. } if graph => {
                    Some((remote.clone(), mailbox.clone()))
                }
                RemoteRef::Imap { mailbox, .. } if !graph => {
                    Some((remote.clone(), mailbox.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| {
                RuntimeError::Proto(mail_proto::ProtoError::Unsupported(
                    "fetching a message that has no address this account can read".to_owned(),
                ))
            })?;
        let mailbox = MailboxRef {
            account: self.account.clone(),
            path,
        };
        let outcome = self
            .run(
                ProtoOp::FetchBody {
                    remotes: vec![remote],
                },
                cancel,
            )
            .await?;
        let ProtoOutcome::Fetched { items, .. } = outcome else {
            return Err(RuntimeError::Proto(mail_proto::ProtoError::Malformed(
                "a body fetch answered with something else".to_owned(),
            )));
        };
        if self.store_bodies(items, &mailbox, now)? == 0 {
            return Err(RuntimeError::Proto(mail_proto::ProtoError::Refused {
                kind: mail_proto::machine::Refusal::Permanent,
                text: "the server no longer has that message".to_owned(),
            }));
        }
        Ok(())
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
                self.account.clone(),
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

    /// Fetch attachments that earlier passes left on the server, for an account kept offline in
    /// full: smallest first, so the largest come last, within `budget`.
    ///
    /// The rebuilt message stays as it was stored (F165): each part is fetched and held exactly
    /// as opening it would ([`AccountEngine::fetch_part`]). A part the server will not give is
    /// named and passed over, so one bad part cannot hold the rest back pass after pass; a
    /// refused sign-in, a rate limit or a dropped connection ends the step, since every part
    /// after it would meet the same.
    pub async fn fetch_remote_parts(
        &mut self,
        mailbox: &MailboxRef,
        cancel: &mut Cancel,
        budget: PartBudget,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        let wanted = self.store.remote_parts_in(mailbox, budget.parts)?;
        for part in budget.take(wanted) {
            match self.fetch_part(part.message, &part.section, cancel).await {
                Ok(_) => report.parts_fetched += 1,
                Err(RuntimeError::Cancelled) => break,
                Err(e) => {
                    let retry = e.retry();
                    report.saw(&retry);
                    report.needs_attention.push(format!(
                        "{}: part {} of a message: {e}",
                        mailbox.path, part.section
                    ));
                    if !matches!(retry, Retry::Fatal(_)) {
                        break;
                    }
                }
            }
        }
        Ok(report)
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
            .put(&bytes)
            .map_err(RuntimeError::Store)?;
        self.store
            .hold_part(message, section, blob, bytes.len() as u64)?;
        Ok(blob)
    }

    /// Which role a folder serves on this account.
    ///
    /// From the capabilities the last `LIST (SPECIAL-USE)` wrote down; see
    /// [`mail_domain::FolderRoles::filed_as`]. POP3's one mailbox is `INBOX` by construction.
    ///
    /// A folder with no role used to fall back to `Inbox`, which was harmless while only the
    /// inbox and Sent were fetched and would have listed every user folder's mail in the inbox
    /// the day they were. It is `Archive` now: kept, and out of the inbox.
    fn role_of(&self, mailbox: &MailboxRef) -> MailboxRole {
        self.caps().folders.filed_as(&mailbox.path)
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
        // Graph's delta already said what changed and what left, in the same pass's sync.
        if self.graph.is_some() {
            return Ok(report);
        }

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
                    self.store.ingest(self.account.clone(), *ingest)?;
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
                    self.store.ingest(self.account.clone(), *ingest)?;
                    self.last.expunges = Some(now);
                }
                Ok(ProtoOutcome::Ingested(mut ingest)) => {
                    // The backend can only say what still exists. What we *hold* is the store's
                    // knowledge, so the diff happens here — and without it `gone` was always
                    // empty and a message deleted on another device never disappeared.
                    ingest.gone = self.vanished(mailbox)?;
                    self.store.ingest(self.account.clone(), *ingest)?;
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
        if self.caps().condstore != Condstore::Qresync {
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
        if !self.caps().condstore.changedsince() {
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
    ///
    /// Only this mailbox's. A fetch selects one mailbox and names UIDs in it, and the account's
    /// whole backlog put the inbox's body pass to asking `INBOX` for UIDs that belonged to Sent.
    ///
    /// The conversations the window is showing come first (`wanted`), and the count of those at
    /// the front is returned beside the list: the caller bands only what follows them, since
    /// what the person is looking at is wanted whatever its size. A message the window is
    /// fetching for itself is left out, so the two do not fetch one body twice.
    fn unfetched(
        &self,
        mailbox: &MailboxRef,
        limit: u32,
    ) -> Result<(Vec<(RemoteRef, u64)>, usize), RuntimeError> {
        // Sizes from this session's survey. A message it did not cover — no survey yet, or a
        // protocol that cannot take one — is "size unknown" and sorts into the last band.
        let sizes: std::collections::HashMap<RemoteRef, u64> =
            self.surveyed().into_iter().collect();
        let (order, first) = crate::wanted::backlog(self.store.as_ref(), mailbox, limit)?;
        let order = order
            .into_iter()
            .map(|remote| {
                let size = sizes.get(&remote).copied().unwrap_or(u64::MAX);
                (remote, size)
            })
            .collect();
        Ok((order, first))
    }

    /// The account's stored credential, refreshed if it is close to expiring.
    pub async fn credential(
        &self,
        purpose: porter_core::SecretPurpose,
    ) -> Result<porter_core::Credential, RuntimeError> {
        self.secrets
            .get(&porter_core::SecretKey {
                account: self.account.clone(),
                purpose,
            })
            .await
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

/// Resolves when `cancel` says stop, or its sender has gone; never otherwise.
async fn cancelled(cancel: &mut Cancel) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
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
                RemoteRef::Imap { .. } | RemoteRef::Graph { .. } | RemoteRef::Jmap { .. } => {
                    unreachable!()
                }
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

    #[test]
    fn a_part_budget_stops_at_its_count_or_its_bytes_and_always_takes_one() {
        let part = |n: u128, size: u64| mail_store::RemotePart {
            message: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
            section: "2".to_owned(),
            size,
        };
        let sizes = |parts: Vec<mail_store::RemotePart>| -> Vec<u64> {
            parts.into_iter().map(|p| p.size).collect()
        };
        let wanted = vec![part(1, 10), part(2, 20), part(3, 30), part(4, 40)];
        const CASES: &[(u32, u64, &[u64])] = &[
            (10, 1_000, &[10, 20, 30, 40]),
            (2, 1_000, &[10, 20]),
            (10, 60, &[10, 20, 30]),
            (10, 59, &[10, 20]),
            // A part larger than the whole byte bound is still fetched, alone.
            (10, 5, &[10]),
        ];
        for (parts, bytes, want) in CASES {
            let budget = PartBudget {
                parts: *parts,
                bytes: *bytes,
            };
            assert_eq!(sizes(budget.take(wanted.clone())), *want, "{budget:?}");
        }
    }
}
