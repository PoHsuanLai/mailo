//! A JMAP account (RFC 8620, RFC 8621): sync, outbox and push, over HTTPS.
//!
//! # Why not an `AccountEngine<JmapBackend>`
//!
//! [`crate::AccountEngine`] drives a [`mail_proto::Backend`] over a [`crate::Transport`]: a TCP
//! socket, TLS, and a sans-I/O machine fed bytes. JMAP has no byte-level session to replay — it
//! is JSON over HTTP, and the HTTP is `reqwest`'s — so a backend would be an HTTP/1.1 client
//! written again inside `mail-proto`, which is the wrong trade. What *is* protocol is still pure
//! and still in `mail-proto` (`mail_proto::jmap`); this engine is the effectful half, the way
//! Graph sending is.
//!
//! What the store sees is the same as from any engine, and that is the part that matters:
//!
//! - messages arrive through [`crate::assemble`] from their real RFC 5322 bytes — headers first
//!   (the email's own header fields, raw), bodies behind — so search, threading, receipts and
//!   invitations read them exactly as they read IMAP mail;
//! - flags and labels arrive as an [`mail_domain::Ingest`], filing through
//!   [`mail_store::Store::refile`], deletions as `gone`;
//! - the cursor is a [`SyncCursor::Jmap`] under the account's one mailbox, [`JMAP_ALL`];
//! - the outbox drains the same `ProtoOp`s, in order, with the same settle and undo rules;
//! - the wait between passes is push where the server offers it, raced against the outbox.
//!
//! # One mailbox, many memberships
//!
//! An email is one object in JMAP whatever mailboxes it sits in, and `Email/changes` reports
//! changes across all of them at once. So the account is synced as one mailbox, every address is
//! a [`RemoteRef::Jmap`] under [`JMAP_ALL`], and mailbox membership becomes what this client
//! already has for Gmail: a role (Inbox, Archive, Sent, Trash) from the mailboxes that have one,
//! and labels for those that do not.

mod client;
mod outbox;
mod push;
mod sync;

pub use client::{Auth, Client, find_session, safe_url};

use crate::{RuntimeError, Secrets, SyncReport};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, AccountPlan, ArchiveMeans, Condstore, ConnectionBudget, Credential,
    ExpungeMeans, HttpAuth, Incoming, JMAP_ALL, MailboxRef, MailboxRole, MoveExt, RemoteRef, Retry,
    Retryable, SecretKey, SecretPurpose, ServerLabels, ServerThreads, Supported, SyncCursor,
    WatchMode,
};
use mail_proto::jmap::{Identity, Mailboxes};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

/// Most headers one pass fetches; the rest wait for the next, like an IMAP folder's.
pub const HEADERS_PER_PASS: usize = 200;
/// Most bodies one pass fetches.
pub const BODIES_PER_PASS: usize = 100;

/// Drives one JMAP account.
pub struct JmapEngine {
    account: AccountId,
    plan: AccountPlan,
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    http: reqwest::Client,
    /// The session, once fetched. Dropped when the server refuses the credential or says the
    /// session changed, so the next use fetches it again.
    client: Option<Client>,
    /// The mailbox list, kept between passes of a watch so an unchanged one is not refetched.
    mailboxes: Option<Mailboxes>,
    identities: Option<Vec<Identity>>,
    /// How often a waiting watch looks in the outbox for a send that has come due.
    outbox_every: Duration,
}

impl std::fmt::Debug for JmapEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapEngine")
            .field("account", &self.account)
            .field("address", &self.plan.address)
            .finish_non_exhaustive()
    }
}

impl JmapEngine {
    pub fn new(
        account: AccountId,
        plan: AccountPlan,
        store: Arc<SqliteStore>,
        secrets: Arc<dyn Secrets>,
    ) -> Result<Self, RuntimeError> {
        Ok(Self {
            account,
            plan,
            store,
            secrets,
            http: crate::signin::http_client()?,
            client: None,
            mailboxes: None,
            identities: None,
            outbox_every: Duration::from_secs(15),
        })
    }

    /// The one mailbox every address of this account is held under.
    pub fn mailbox(&self) -> MailboxRef {
        MailboxRef {
            account: self.account,
            path: JMAP_ALL.to_owned(),
        }
    }

    /// Where the session lives, from the plan.
    fn session_url(&self) -> Result<(&str, HttpAuth), RuntimeError> {
        match &self.plan.incoming {
            Incoming::Jmap { session, auth } => Ok((session, *auth)),
            _ => Err(RuntimeError::UnsupportedIo("not a JMAP account".to_owned())),
        }
    }

    /// The credential, as the header the plan says it travels in.
    fn auth(&self) -> Result<Auth, RuntimeError> {
        let (_, how) = self.session_url()?;
        let credential = self.secrets.get(&SecretKey {
            account: self.account,
            purpose: SecretPurpose::IncomingPassword,
        })?;
        Ok(match (credential, how) {
            (Credential::Password(password), HttpAuth::Basic) => Auth::Basic {
                username: self.plan.username(),
                password,
            },
            (Credential::Password(token), HttpAuth::Bearer) => Auth::Bearer(token),
            (Credential::OAuth { access, .. }, _) => Auth::Bearer(access),
            // A key kept for signing or decrypting is not a sign-in, and is never sent.
            (Credential::OpenPgp(_) | Credential::SmimeKey(_), _) => {
                return Err(RuntimeError::Secrets(
                    "the credential kept for this account is a key, not a password or token"
                        .to_owned(),
                ));
            }
        })
    }

    /// The session, fetching it if this engine has none.
    async fn client(&mut self) -> Result<&Client, RuntimeError> {
        if self.client.is_none() {
            let (url, _) = self.session_url()?;
            let url = url.to_owned();
            let client = Client::connect(&self.http, &url, self.auth()?).await?;
            self.client = Some(client);
        }
        self.client
            .as_ref()
            .ok_or_else(|| RuntimeError::Connect("JMAP: no session".to_owned()))
    }

    /// Forget the session after a failure that may mean it is stale: a refused credential, or
    /// a server that answers with a different session state.
    fn forget_if_stale(&mut self, error: &RuntimeError) {
        if matches!(error.retry(), Retry::NeedsReauth) {
            self.client = None;
        }
    }

    /// Check reachability and the credential: fetch the session.
    pub async fn connect(&mut self) -> Result<(), RuntimeError> {
        match self.client().await {
            Ok(_) => Ok(()),
            Err(e) => {
                self.forget_if_stale(&e);
                Err(e)
            }
        }
    }

    /// What the server supports, from the session and the mailbox list.
    fn caps(&self, mailboxes: &Mailboxes, now: DateTime<Utc>) -> AccountCaps {
        let roles = mailboxes.roles();
        let push = self
            .client
            .as_ref()
            .is_some_and(|c| c.session.event_source_url.is_some());
        AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: if push {
                WatchMode::Idle
            } else {
                WatchMode::Poll {
                    every: Duration::from_secs(300),
                }
            },
            // Where there is no Archive mailbox, archiving is leaving the inbox: the other
            // mailboxes an email is in are labels, as on a server whose folders are labels.
            archive: match roles.path(MailboxRole::Archive) {
                Some(path) => ArchiveMeans::MoveToFolder(path.to_owned()),
                None => ArchiveMeans::DropInbox,
            },
            folders: roles,
            condstore: Condstore::Absent,
            // Moving is a patch of `mailboxIds`: atomic, and never a copy and a delete.
            move_ext: MoveExt::Supported,
            // Nothing is destroyed but mail the user put in the Trash and emptied; see `outbox`.
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget { max: 2 },
            observed_at: now,
        }
    }

    /// The stored cursor, if this account has one.
    fn cursor(&self) -> Result<Option<(String, String)>, RuntimeError> {
        Ok(match self.store.cursor(&self.mailbox())? {
            Some(SyncCursor::Jmap {
                email_state,
                mailbox_state,
            }) => Some((email_state, mailbox_state)),
            _ => None,
        })
    }

    /// One whole pass: the mailbox list, what changed, headers, bodies, then the outbox.
    ///
    /// Failures after the session is fetched are folded into the report rather than returned,
    /// like an IMAP pass's: a body that will not download is worth seeing and is no reason to
    /// abandon the headers already in hand. A refused credential ends the pass at once and says
    /// so, so a watch stops instead of retrying it every minute.
    pub async fn pass(
        &mut self,
        cancel: &mut crate::Cancel,
        now: DateTime<Utc>,
    ) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        if let Err(e) = self.connect().await {
            if matches!(e.retry(), Retry::NeedsReauth) {
                report.saw(&Retry::NeedsReauth);
                report.needs_attention.push(e.to_string());
                return Ok(report);
            }
            return Err(e);
        }
        let steps: [Step; 3] = [Step::Mail, Step::Bodies, Step::Outbox];
        for step in steps {
            if *cancel.borrow() {
                break;
            }
            let done = match step {
                Step::Mail => self.sync(now, HEADERS_PER_PASS, &mut report).await,
                Step::Bodies => self.fetch_bodies(now, BODIES_PER_PASS, &mut report).await,
                Step::Outbox => self.drain_into(now, &mut report).await,
            };
            if let Err(e) = done {
                self.forget_if_stale(&e);
                let retry = e.retry();
                report.saw(&retry);
                report.needs_attention.push(e.to_string());
                if matches!(retry, Retry::NeedsReauth) {
                    break;
                }
            }
        }
        Ok(report)
    }

    /// The outbox alone, as `mailo import --to-mailbox` wants it after queueing uploads.
    pub async fn drain_outbox(&mut self, now: DateTime<Utc>) -> Result<SyncReport, RuntimeError> {
        let mut report = SyncReport::default();
        self.drain_into(now, &mut report).await?;
        Ok(report)
    }
}

/// The parts of a pass, in order.
#[derive(Debug, Clone, Copy)]
enum Step {
    Mail,
    Bodies,
    Outbox,
}

/// The ids among `remotes` that are JMAP emails, each once.
fn email_ids(remotes: &[RemoteRef]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for remote in remotes {
        if let RemoteRef::Jmap { email_id } = remote
            && !ids.contains(email_id)
        {
            ids.push(email_id.clone());
        }
    }
    ids
}
