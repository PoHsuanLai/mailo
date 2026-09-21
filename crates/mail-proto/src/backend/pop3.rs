//! POP3 as a [`Backend`]: product operations to a command walk and back to domain values.

use crate::machine::{Backend, IoReady, Machine, Progress, ProtoError, ProtoOutcome};
use crate::pop3::{Pop3Command, Pop3Reply, Pop3Session};
use mail_domain::{
    AccountCaps, AccountId, FetchSince, Ingest, MailboxRef, ProtoOp, RemoteRef, SyncCursor,
    UidValidity,
};

/// What the backend is in the middle of.
///
/// An explicit state rather than flags, so the reply handler can only ever be interpreting a
/// reply to something it actually asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Idle,
    /// `CAPA`, to learn whether `TOP` and `PIPELINING` are real.
    Caps,
    /// Map the maildrop: `STAT`, `UIDL`, `LIST`.
    Survey {
        mailbox: MailboxRef,
    },
    /// Headers for one message via `TOP n 0`.
    Headers {
        remote: RemoteRef,
    },
    /// A whole message via `RETR`.
    Body {
        remote: RemoteRef,
    },
}

/// Builds a session for one command walk.
///
/// A closure rather than credentials on the backend: POP3 gives its commands at construction,
/// so the backend would otherwise have to hold the password to build each session. This way it
/// never sees one and therefore cannot leak one.
///
/// **The factory is responsible for authentication.** It prepends whatever the account's
/// `AuthPlan` calls for — `USER`/`PASS`, or `AUTH PLAIN` — before the commands it is handed,
/// because it is the only thing that knows both the mechanism and the credential. The backend
/// asks for `STAT`; what reaches the wire is an authenticated session that then runs `STAT`.
pub type SessionFactory =
    Box<dyn FnMut(Authenticate, Vec<Pop3Command>) -> Result<Pop3Session, ProtoError> + Send>;

/// Whether a command walk needs an authenticated session.
///
/// `CAPA` is legal in the AUTHORIZATION state and RFC 2449 says the answer **may differ** once
/// authenticated — the same trap F14 caught on Gmail's IMAP, on another protocol. So a
/// capability read asks twice and believes the second answer, and that is the one walk that
/// starts unauthenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authenticate {
    /// Prepend the account's auth commands before these.
    First,
    /// Run these as given; the walk authenticates itself, or does not need to.
    No,
}

/// Drives [`Pop3Session`] on behalf of an account.
///
/// `Debug` is written by hand because the session factory is a closure holding the account's
/// credential; a derived one would not compile, and making the closure `Debug` would put a
/// password one `{:?}` away from a log.
pub struct Pop3Backend {
    account: AccountId,
    caps: AccountCaps,
    build: SessionFactory,
    session: Option<Pop3Session>,
    job: Job,
    /// `UIDL` and `LIST` collected during a survey, keyed by server message number.
    uidls: Vec<(u32, String)>,
    sizes: Vec<(u32, u64)>,
}

impl Pop3Backend {
    /// A backend over an already-constructed session.
    ///
    /// The session carries the credentials, which is why it is built by the runtime — this type
    /// never sees a password and so cannot leak one.
    pub fn new(account: AccountId, caps: AccountCaps, build: SessionFactory) -> Self {
        Self {
            account,
            caps,
            build,
            session: None,
            job: Job::Idle,
            uidls: Vec::new(),
            sizes: Vec::new(),
        }
    }

    /// Everything learned by the last survey: UIDL, size, oldest first.
    ///
    /// The runtime uses this to order the first sync — smallest band first, newest first — which
    /// is the difference between a usable inbox in a minute and one in twenty.
    pub fn surveyed(&self) -> Vec<(String, u64)> {
        self.uidls
            .iter()
            .map(|(number, uidl)| {
                let size = self
                    .sizes
                    .iter()
                    .find(|(n, _)| n == number)
                    .map(|(_, octets)| *octets)
                    .unwrap_or(0);
                (uidl.clone(), size)
            })
            .collect()
    }

    fn number_for(&self, uidl: &str) -> Option<u32> {
        self.uidls
            .iter()
            .find(|(_, u)| u == uidl)
            .map(|(number, _)| *number)
    }

    /// An `Ingest` carrying nothing but a cursor, for operations POP3 answers locally.
    fn empty_ingest(&self, mailbox: MailboxRef) -> Ingest {
        Ingest {
            mailbox,
            // POP3 has no UIDVALIDITY. A UIDL change is detected by diffing the survey against
            // remote_map, not announced by the server.
            validity: UidValidity::Same,
            cursor: SyncCursor::Pop,
            messages: Vec::new(),
            flags: Vec::new(),
            labels: Vec::new(),
            gone: Vec::new(),
        }
    }

    fn queue(&mut self, auth: Authenticate, commands: Vec<Pop3Command>) -> Progress<ProtoOutcome> {
        let session = match (self.build)(auth, commands) {
            Ok(session) => session,
            Err(e) => return Progress::Failed(e),
        };
        self.session = Some(session);
        let session = self.session.as_mut().expect("just assigned");
        match session.start() {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Done(_) => Progress::Failed(ProtoError::Malformed(
                "session finished before the backend asked for anything".to_owned(),
            )),
            Progress::Failed(e) => Progress::Failed(e),
        }
    }
}

impl Backend for Pop3Backend {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        match op {
            ProtoOp::FetchCaps => {
                self.job = Job::Caps;
                // Ask, authenticate, ask again. RFC 2449 permits the two answers to differ, and
                // the second is the one that describes the session we will actually use.
                self.queue(
                    Authenticate::No,
                    vec![
                        Pop3Command::Capa,
                        Pop3Command::User,
                        Pop3Command::Pass,
                        Pop3Command::Capa,
                    ],
                )
            }
            ProtoOp::FetchEnvelopes { mailbox, since } => {
                self.job = Job::Survey {
                    mailbox: mailbox.clone(),
                };
                self.uidls.clear();
                self.sizes.clear();
                // POP3 cannot fetch incrementally: there is no server-side "since". The survey
                // is the whole maildrop either way, and the caller diffs it against remote_map.
                let _ = matches!(since, FetchSince::Beginning);
                self.queue(
                    Authenticate::First,
                    vec![Pop3Command::Stat, Pop3Command::Uidl, Pop3Command::List],
                )
            }
            ProtoOp::FetchHeaders { remote } => {
                let RemoteRef::Pop { ref uidl } = remote else {
                    return Progress::Failed(ProtoError::Unsupported(
                        "an IMAP reference cannot be fetched over POP3".to_owned(),
                    ));
                };
                let Some(number) = self.number_for(uidl) else {
                    return Progress::Failed(ProtoError::Refused {
                        kind: crate::machine::Refusal::Permanent,
                        text: format!("{uidl} is not in the current survey"),
                    });
                };
                if !self.caps.top.usable() {
                    // Falling back to RETR here would silently mark the message read, which is
                    // the exact harm this operation exists to avoid. Say so instead.
                    return Progress::Failed(ProtoError::Unsupported(
                        "TOP is unavailable, and RETR would mark the message read".to_owned(),
                    ));
                }
                self.job = Job::Headers {
                    remote: remote.clone(),
                };
                // Zero body lines: every documented TOP bug in the wild is a failure to return
                // the requested body LINES, never wrong headers.
                self.queue(Authenticate::First, vec![Pop3Command::Top(number, 0)])
            }
            ProtoOp::FetchBody { remote } => {
                let RemoteRef::Pop { ref uidl } = remote else {
                    return Progress::Failed(ProtoError::Unsupported(
                        "an IMAP reference cannot be fetched over POP3".to_owned(),
                    ));
                };
                let Some(number) = self.number_for(uidl) else {
                    // Message numbers are session-scoped, so a stale one addresses the wrong
                    // message. Refusing is the only safe answer.
                    return Progress::Failed(ProtoError::Refused {
                        kind: crate::machine::Refusal::Permanent,
                        text: format!("{uidl} is not in the current survey"),
                    });
                };
                self.job = Job::Body {
                    remote: remote.clone(),
                };
                self.queue(Authenticate::First, vec![Pop3Command::Retr(number)])
            }
            // Everything below has no POP3 representation. Confirming immediately is correct
            // rather than a stub: the local change IS the whole change.
            ProtoOp::SetFlags { .. }
            | ProtoOp::SetMailbox { .. }
            | ProtoOp::SetLabels { .. }
            | ProtoOp::Expunge { .. } => {
                self.job = Job::Idle;
                Progress::Done(ProtoOutcome::Applied)
            }
            ProtoOp::ListFolders => {
                // One implicit maildrop; there is nothing to list.
                self.job = Job::Idle;
                Progress::Done(ProtoOutcome::Applied)
            }
            ProtoOp::Watch { .. } => {
                // POP3 has no push. The runtime polls, so a watch completes at once and the
                // interval lives in AccountCaps::watch.
                self.job = Job::Idle;
                Progress::Done(ProtoOutcome::Woken)
            }
            ProtoOp::Append { .. } | ProtoOp::Submit { .. } => Progress::Failed(
                ProtoError::Unsupported("POP3 cannot store or send a message".to_owned()),
            ),
        }
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        let Some(session) = self.session.as_mut() else {
            return Progress::Failed(ProtoError::Malformed(
                "bytes arrived with no session in flight".to_owned(),
            ));
        };
        let replies = match session.feed(ready) {
            Progress::Need(needs) => return Progress::Need(needs),
            Progress::Failed(e) => return Progress::Failed(e),
            Progress::Done(replies) => replies,
        };

        match std::mem::replace(&mut self.job, Job::Idle) {
            Job::Idle => Progress::Failed(ProtoError::Malformed(
                "a reply arrived with no operation in flight".to_owned(),
            )),
            Job::Caps => {
                let mut caps = self.caps.clone();
                // Iterating in order means the POST-authentication CAPA wins, which is the
                // whole point of asking twice.
                for reply in &replies {
                    if let Pop3Reply::Capabilities(lines) = reply {
                        caps.top = supported(lines, "TOP");
                        caps.pipelining = supported(lines, "PIPELINING");
                    }
                    // CAPA answered -ERR leaves both Absent, which is the honest reading: an
                    // old server that does not implement CAPA may still implement TOP, but we
                    // will not find out by guessing.
                }
                self.caps = caps.clone();
                Progress::Done(ProtoOutcome::Caps(Box::new(caps)))
            }
            Job::Survey { mailbox } => {
                for reply in &replies {
                    match reply {
                        Pop3Reply::Uidl(rows) => {
                            self.uidls = rows.iter().map(|r| (r.number, r.uidl.clone())).collect();
                        }
                        Pop3Reply::List(rows) => {
                            self.sizes = rows.iter().map(|r| (r.number, r.octets)).collect();
                        }
                        _ => {}
                    }
                }
                Progress::Done(ProtoOutcome::Ingested(Box::new(self.empty_ingest(mailbox))))
            }
            Job::Headers { remote } => {
                let raw = replies.iter().find_map(|reply| match reply {
                    Pop3Reply::Headers(bytes) => Some(bytes.clone()),
                    _ => None,
                });
                match raw {
                    Some(bytes) => Progress::Done(ProtoOutcome::Fetched { remote, raw: bytes }),
                    None => Progress::Failed(ProtoError::Malformed(
                        "TOP completed without headers".to_owned(),
                    )),
                }
            }
            Job::Body { remote } => {
                let raw = replies.iter().find_map(|reply| match reply {
                    Pop3Reply::Retrieved(bytes) => Some(bytes.clone()),
                    _ => None,
                });
                match raw {
                    // Parsing into a Message needs ids the protocol cannot supply, so the raw
                    // bytes go back and the runtime assembles. That keeps mail-mime out of this
                    // crate's hot path and the blob store out of its knowledge entirely.
                    Some(bytes) => Progress::Done(ProtoOutcome::Fetched { remote, raw: bytes }),
                    None => Progress::Failed(ProtoError::Malformed(
                        "RETR completed without a message".to_owned(),
                    )),
                }
            }
        }
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }
}

/// Whether a `CAPA` listing advertises `name`.
///
/// Advertisement only. `AccountCaps::Supported::Withdrawn` is for what we observe afterwards,
/// and nothing here may set it — a capability is withdrawn by misbehaviour, not by a re-read.
fn supported(lines: &[String], name: &str) -> mail_domain::Supported {
    let present = lines
        .iter()
        .any(|line| line.split_whitespace().next() == Some(name));
    if present {
        mail_domain::Supported::Yes
    } else {
        mail_domain::Supported::Absent
    }
}

/// The account this backend serves.
impl Pop3Backend {
    pub fn account(&self) -> AccountId {
        self.account
    }
}

impl std::fmt::Debug for Pop3Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pop3Backend")
            .field("account", &self.account)
            .field("job", &self.job)
            .field("surveyed", &self.uidls.len())
            .field("session", &self.session.is_some())
            .finish()
    }
}
