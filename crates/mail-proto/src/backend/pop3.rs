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
    /// Headers via `TOP n 0`, one command per message on one connection.
    Headers {
        remotes: Vec<RemoteRef>,
    },
    /// Whole messages via `RETR`, one command per message on one connection.
    Body {
        remotes: Vec<RemoteRef>,
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

pub use super::Authenticate;

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

    /// One command per message, or `None` if any of them is not in the current survey.
    ///
    /// All-or-nothing: message numbers are session-scoped, so a stale one addresses a different
    /// message, and fetching the wrong message is worse than fetching none.
    fn numbers_for(
        &self,
        remotes: &[RemoteRef],
        command: impl Fn(u32) -> Pop3Command,
    ) -> Option<Vec<Pop3Command>> {
        remotes
            .iter()
            .map(|remote| match remote {
                RemoteRef::Pop { uidl } => self.number_for(uidl).map(&command),
                RemoteRef::Imap { .. } => None,
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
            cursor: Some(SyncCursor::Pop),
            messages: Vec::new(),
            flags: Vec::new(),
            labels: Vec::new(),
            label_names: Vec::new(),
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
            ProtoOp::FetchHeaders { remotes } => {
                if !self.caps.top.usable() {
                    // Falling back to RETR would silently mark every message read, which is the
                    // exact harm this operation exists to avoid.
                    return Progress::Failed(ProtoError::Unsupported(
                        "TOP is unavailable, and RETR would mark the message read".to_owned(),
                    ));
                }
                let Some(commands) = self.numbers_for(&remotes, |n| Pop3Command::Top(n, 0)) else {
                    return Progress::Failed(ProtoError::Refused {
                        kind: crate::machine::Refusal::Permanent,
                        text: "a message is not in the current survey".to_owned(),
                    });
                };
                self.job = Job::Headers { remotes };
                self.queue(Authenticate::First, commands)
            }
            ProtoOp::FetchBody { remotes } => {
                let Some(commands) = self.numbers_for(&remotes, Pop3Command::Retr) else {
                    return Progress::Failed(ProtoError::Refused {
                        kind: crate::machine::Refusal::Permanent,
                        text: "a message is not in the current survey".to_owned(),
                    });
                };
                self.job = Job::Body { remotes };
                self.queue(Authenticate::First, commands)
            }
            // Everything below has no POP3 representation. Confirming immediately is correct
            // rather than a stub: the local change IS the whole change.
            ProtoOp::SetFlags { .. }
            | ProtoOp::SetMailbox { .. }
            | ProtoOp::SetLabels { .. }
            | ProtoOp::File { .. }
            | ProtoOp::AddKeyword { .. }
            | ProtoOp::Expunge { .. } => {
                self.job = Job::Idle;
                Progress::Done(ProtoOutcome::Applied)
            }
            ProtoOp::FetchStructure { .. } | ProtoOp::FetchSections { .. } => {
                // `TOP` takes a line count, not a part: a POP3 message comes whole or not at
                // all, so a large one is simply a large download.
                Progress::Failed(ProtoError::Unsupported(
                    "fetching part of a message over POP3".to_owned(),
                ))
            }
            ProtoOp::FetchFlags { .. } => {
                // POP3 has no flags on the server at all — read and starred live only here — so
                // there is nothing to sweep for and nothing to reconcile.
                self.job = Job::Idle;
                Progress::Done(ProtoOutcome::Applied)
            }
            ProtoOp::ListRemote { mailbox, .. } => {
                // The expunge diff. On POP3 this is the survey: UIDL is the complete list of
                // what the server still holds, and anything in remote_map that is missing from
                // it was deleted elsewhere.
                self.job = Job::Survey {
                    mailbox: mailbox.clone(),
                };
                self.uidls.clear();
                self.sizes.clear();
                self.queue(Authenticate::First, vec![Pop3Command::Uidl])
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
            // Refused before it is ever queued (`mail_domain::folder::plan`); this is the
            // backstop for a row that got into the outbox some other way.
            ProtoOp::Folder(_) => Progress::Failed(ProtoError::Unsupported(
                "POP3 has one mailbox; it has no folders to change".to_owned(),
            )),
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
            Job::Headers { remotes } => {
                let bodies: Vec<Vec<u8>> = replies
                    .iter()
                    .filter_map(|reply| match reply {
                        Pop3Reply::Headers(bytes) => Some(bytes.clone()),
                        _ => None,
                    })
                    .collect();
                Progress::Done(ProtoOutcome::Fetched {
                    // POP3 has no server-side flags; read and starred live only here.
                    flags: Vec::new(),
                    items: remotes.into_iter().zip(bodies).collect(),
                })
            }
            Job::Body { remotes } => {
                // Parsing into a Message needs ids the protocol cannot supply, so the raw bytes
                // go back and the runtime assembles. That keeps mail-mime out of this crate and
                // the blob store out of its knowledge entirely.
                let bodies: Vec<Vec<u8>> = replies
                    .iter()
                    .filter_map(|reply| match reply {
                        Pop3Reply::Retrieved(bytes) => Some(bytes.clone()),
                        _ => None,
                    })
                    .collect();
                Progress::Done(ProtoOutcome::Fetched {
                    // POP3 has no server-side flags; read and starred live only here.
                    flags: Vec::new(),
                    items: remotes.into_iter().zip(bodies).collect(),
                })
            }
        }
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }

    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        self.uidls
            .iter()
            .map(|(number, uidl)| {
                let size = self
                    .sizes
                    .iter()
                    .find(|(n, _)| n == number)
                    .map(|(_, octets)| *octets)
                    .unwrap_or(u64::MAX);
                (RemoteRef::Pop { uidl: uidl.clone() }, size)
            })
            .collect()
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
