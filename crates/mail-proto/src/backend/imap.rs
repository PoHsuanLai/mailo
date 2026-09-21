//! IMAP as a [`Backend`].
//!
//! Submission is deliberately absent: it is a separate connection to a separate host and lives
//! in [`crate::backend::SmtpBackend`].

use crate::imap::{ImapCommand, ImapSession};
use crate::machine::{Backend, IoReady, Machine, Progress, ProtoError, ProtoOutcome};
use crate::mutf7;
use mail_domain::{
    AccountCaps, AccountId, ArchiveMeans, Condstore, ExpungeMeans, FetchSince, FolderRoles, Ingest,
    MailboxRef, MailboxRole, MoveExt, ProtoOp, RemoteRef, ServerLabels, SyncCursor, UidValidity,
};

/// Builds a session for one command walk, owning the credential so the backend never sees it.
pub type SessionFactory =
    Box<dyn FnMut(Vec<ImapCommand>) -> Result<ImapSession, ProtoError> + Send>;

/// What the backend is in the middle of.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Idle,
    Caps,
    Folders,
    Envelopes { mailbox: MailboxRef },
    Flags { mailbox: MailboxRef },
    Listing { mailbox: MailboxRef },
    Fetch { remote: RemoteRef },
    Applied,
    Watching,
}

/// Drives [`ImapSession`] on behalf of an account.
pub struct ImapBackend {
    account: AccountId,
    caps: AccountCaps,
    build: SessionFactory,
    session: Option<ImapSession>,
    job: Job,
}

impl ImapBackend {
    pub fn new(account: AccountId, caps: AccountCaps, build: SessionFactory) -> Self {
        Self {
            account,
            caps,
            build,
            session: None,
            job: Job::Idle,
        }
    }

    pub fn account(&self) -> AccountId {
        self.account
    }

    fn queue(&mut self, commands: Vec<ImapCommand>) -> Progress<ProtoOutcome> {
        let session = match (self.build)(commands) {
            Ok(session) => session,
            Err(e) => return Progress::Failed(e),
        };
        self.session = Some(session);
        match self.session.as_mut().expect("just assigned").start() {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Done(_) => Progress::Failed(ProtoError::Malformed(
                "session finished before doing anything".to_owned(),
            )),
            Progress::Failed(e) => Progress::Failed(e),
        }
    }

    /// Select a mailbox read-only unless the walk needs to change something.
    ///
    /// `EXAMINE` rather than `SELECT` wherever possible: a read-only selection cannot set
    /// `\Recent` or implicitly expunge, and fetching should never have a side effect.
    fn select(mailbox: &MailboxRef, read_only: bool) -> ImapCommand {
        ImapCommand::Select {
            mailbox: mailbox.path.clone(),
            read_only,
        }
    }

    fn empty_ingest(&self, mailbox: MailboxRef, validity: UidValidity) -> Ingest {
        Ingest {
            mailbox,
            validity,
            cursor: SyncCursor::Imap {
                uidvalidity: 0,
                uidnext: 0,
                modseq: None,
            },
            messages: Vec::new(),
            flags: Vec::new(),
            labels: Vec::new(),
            gone: Vec::new(),
        }
    }
}

impl Backend for ImapBackend {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        match op {
            ProtoOp::FetchCaps => {
                self.job = Job::Caps;
                // Ask, authenticate, ask again. Gmail's pre-auth list omits CONDSTORE, MOVE and
                // SPECIAL-USE, so believing the first answer reports a far less capable server
                // than it is — F14, measured against a real account.
                self.queue(vec![
                    ImapCommand::Capability,
                    ImapCommand::AuthenticateXoauth2,
                    ImapCommand::Capability,
                ])
            }
            ProtoOp::ListFolders => {
                self.job = Job::Folders;
                self.queue(vec![ImapCommand::AuthenticateXoauth2, ImapCommand::List])
            }
            ProtoOp::FetchEnvelopes { mailbox, since } => {
                let set = match since {
                    FetchSince::Beginning => "1:*".to_owned(),
                    FetchSince::After { cursor } => match cursor {
                        SyncCursor::Imap { uidnext, .. } => format!("{uidnext}:*"),
                        // A POP cursor on an IMAP mailbox is a caller bug, not a fetch range.
                        SyncCursor::Pop => {
                            return Progress::Failed(ProtoError::Malformed(
                                "a POP cursor cannot resume an IMAP mailbox".to_owned(),
                            ));
                        }
                    },
                };
                self.job = Job::Envelopes {
                    mailbox: mailbox.clone(),
                };
                let items = if matches!(self.caps.labels, ServerLabels::Supported) {
                    "(UID FLAGS INTERNALDATE ENVELOPE BODYSTRUCTURE X-GM-MSGID X-GM-THRID X-GM-LABELS)"
                } else {
                    "(UID FLAGS INTERNALDATE ENVELOPE BODYSTRUCTURE)"
                };
                self.queue(vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, true),
                    ImapCommand::UidFetch {
                        set,
                        items: items.to_owned(),
                    },
                ])
            }
            ProtoOp::FetchHeaders { remote } => match remote {
                RemoteRef::Imap {
                    ref mailbox,
                    uid,
                    uidvalidity,
                } => {
                    let reference = MailboxRef {
                        account: self.account,
                        path: mailbox.clone(),
                    };
                    self.job = Job::Fetch {
                        remote: remote.clone(),
                    };
                    let _ = uidvalidity;
                    // BODY.PEEK, never BODY: the peeking form does not set \Seen. This is IMAP's
                    // equivalent of POP3's TOP, and fetching headers with BODY would mark every
                    // message read as a side effect of populating a list.
                    self.queue(vec![
                        ImapCommand::AuthenticateXoauth2,
                        Self::select(&reference, true),
                        ImapCommand::UidFetch {
                            set: uid.to_string(),
                            items: "(UID FLAGS BODY.PEEK[HEADER])".to_owned(),
                        },
                    ])
                }
                RemoteRef::Pop { .. } => Progress::Failed(ProtoError::Unsupported(
                    "a POP reference cannot be fetched over IMAP".to_owned(),
                )),
            },
            ProtoOp::FetchBody { remote } => match remote {
                RemoteRef::Imap {
                    ref mailbox, uid, ..
                } => {
                    let reference = MailboxRef {
                        account: self.account,
                        path: mailbox.clone(),
                    };
                    self.job = Job::Fetch {
                        remote: remote.clone(),
                    };
                    self.queue(vec![
                        ImapCommand::AuthenticateXoauth2,
                        Self::select(&reference, true),
                        ImapCommand::UidFetch {
                            set: uid.to_string(),
                            items: "(UID BODY.PEEK[])".to_owned(),
                        },
                    ])
                }
                RemoteRef::Pop { .. } => Progress::Failed(ProtoError::Unsupported(
                    "a POP reference cannot be fetched over IMAP".to_owned(),
                )),
            },
            ProtoOp::SetFlags {
                remotes,
                read,
                star,
            } => {
                let Some(set) = uid_set(&remotes) else {
                    return Progress::Done(ProtoOutcome::Applied);
                };
                let mut add = Vec::new();
                let mut remove = Vec::new();
                match read {
                    Some(mail_domain::ReadState::Read) => add.push("\\Seen"),
                    Some(mail_domain::ReadState::Unread) => remove.push("\\Seen"),
                    None => {}
                }
                match star {
                    Some(mail_domain::Star::Starred) => add.push("\\Flagged"),
                    Some(mail_domain::Star::Unstarred) => remove.push("\\Flagged"),
                    None => {}
                }
                let mailbox = mailbox_of(&remotes, self.account);
                let mut commands = vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, false),
                ];
                if !add.is_empty() {
                    commands.push(ImapCommand::UidStore {
                        set: set.clone(),
                        what: format!("+FLAGS ({})", add.join(" ")),
                    });
                }
                if !remove.is_empty() {
                    commands.push(ImapCommand::UidStore {
                        set,
                        what: format!("-FLAGS ({})", remove.join(" ")),
                    });
                }
                self.job = Job::Applied;
                self.queue(commands)
            }
            ProtoOp::SetLabels {
                remotes,
                add,
                remove,
            } => {
                if !matches!(self.caps.labels, ServerLabels::Supported) {
                    return Progress::Done(ProtoOutcome::Applied);
                }
                let Some(set) = uid_set(&remotes) else {
                    return Progress::Done(ProtoOutcome::Applied);
                };
                let mailbox = mailbox_of(&remotes, self.account);
                let mut commands = vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, false),
                ];
                if !add.is_empty() {
                    commands.push(ImapCommand::UidStore {
                        set: set.clone(),
                        what: format!("+X-GM-LABELS ({})", quoted_labels(&add)),
                    });
                }
                if !remove.is_empty() {
                    commands.push(ImapCommand::UidStore {
                        set,
                        what: format!("-X-GM-LABELS ({})", quoted_labels(&remove)),
                    });
                }
                self.job = Job::Applied;
                self.queue(commands)
            }
            ProtoOp::SetMailbox { remotes, role } => {
                let Some(set) = uid_set(&remotes) else {
                    return Progress::Done(ProtoOutcome::Applied);
                };
                let source = mailbox_of(&remotes, self.account);
                match &self.caps.archive {
                    ArchiveMeans::LocalOnly => Progress::Done(ProtoOutcome::Applied),
                    ArchiveMeans::DropInbox => {
                        // Gmail: filing is label membership. Removing \Inbox archives, and
                        // nothing is deleted.
                        let label = gmail_label(role);
                        self.job = Job::Applied;
                        self.queue(vec![
                            ImapCommand::AuthenticateXoauth2,
                            Self::select(&source, false),
                            ImapCommand::UidStore {
                                set: set.clone(),
                                what: format!("+X-GM-LABELS ({label})"),
                            },
                            ImapCommand::UidStore {
                                set,
                                what: "-X-GM-LABELS (\\Inbox)".to_owned(),
                            },
                        ])
                    }
                    ArchiveMeans::MoveToFolder(target) => {
                        let target = target.clone();
                        self.job = Job::Applied;
                        let mut commands = vec![
                            ImapCommand::AuthenticateXoauth2,
                            Self::select(&source, false),
                        ];
                        // COPY and stop. A move completed by \Deleted + EXPUNGE is forbidden
                        // wherever expunging is: Gmail may be set to deleteForever and we
                        // cannot read that setting, so the copy stands and a stray original is
                        // a cosmetic problem rather than lost mail.
                        commands.push(ImapCommand::UidCopy {
                            set,
                            mailbox: target,
                        });
                        if matches!(self.caps.expunge, ExpungeMeans::Allowed)
                            && matches!(self.caps.move_ext, MoveExt::Supported)
                        {
                            // Only where the server both supports MOVE and permits deletion.
                            // Still never a manual \Deleted dance.
                        }
                        self.queue(commands)
                    }
                }
            }
            ProtoOp::FetchFlags {
                mailbox,
                since_modseq,
            } => {
                self.job = Job::Flags {
                    mailbox: mailbox.clone(),
                };
                let items = match (since_modseq, self.caps.condstore) {
                    (Some(modseq), Condstore::Supported) => {
                        format!("(UID FLAGS) (CHANGEDSINCE {modseq})")
                    }
                    // No modseq, or none we trust: a full flag fetch is slow and correct, which
                    // is the right way round to be wrong. Dovecot 2.0.18 froze HIGHESTMODSEQ at
                    // 1 while EXISTS climbed, and trusting that means never seeing another
                    // flag change.
                    _ => "(UID FLAGS)".to_owned(),
                };
                self.queue(vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, true),
                    ImapCommand::UidFetch {
                        set: "1:*".to_owned(),
                        items,
                    },
                ])
            }
            ProtoOp::ListRemote { mailbox } => {
                self.job = Job::Listing {
                    mailbox: mailbox.clone(),
                };
                // Without QRESYNC — which Gmail does not offer — this is the only way to find
                // what was expunged elsewhere. RFC 7162 says so outright.
                self.queue(vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, true),
                    ImapCommand::UidSearch {
                        criteria: "ALL".to_owned(),
                    },
                ])
            }
            ProtoOp::Watch { mailbox } => {
                self.job = Job::Watching;
                self.queue(vec![
                    ImapCommand::AuthenticateXoauth2,
                    Self::select(&mailbox, true),
                    ImapCommand::Idle,
                ])
            }
            ProtoOp::Expunge { .. } => Progress::Failed(ProtoError::Unsupported(
                "expunging is forbidden: Gmail may be configured to delete permanently, and \
                 that setting cannot be read over IMAP"
                    .to_owned(),
            )),
            ProtoOp::Append { .. } | ProtoOp::Submit { .. } => Progress::Failed(
                ProtoError::Unsupported("submission is a separate backend".to_owned()),
            ),
        }
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        let Some(session) = self.session.as_mut() else {
            return Progress::Failed(ProtoError::Malformed(
                "bytes arrived with no session in flight".to_owned(),
            ));
        };
        let transcript = match session.feed(ready) {
            Progress::Need(needs) => return Progress::Need(needs),
            Progress::Failed(e) => return Progress::Failed(e),
            Progress::Done(transcript) => transcript,
        };

        match std::mem::replace(&mut self.job, Job::Idle) {
            Job::Idle => Progress::Failed(ProtoError::Malformed(
                "a transcript arrived with no operation in flight".to_owned(),
            )),
            Job::Caps => {
                let mut caps = self.caps.clone();
                let seen = |name: &str| {
                    transcript
                        .capabilities
                        .iter()
                        .any(|c| c.to_uppercase().contains(name))
                };
                caps.condstore = if seen("CONDSTORE") {
                    Condstore::Supported
                } else {
                    Condstore::Absent
                };
                caps.move_ext = if seen("MOVE") {
                    MoveExt::Supported
                } else {
                    MoveExt::Absent
                };
                caps.labels = if seen("X-GM-EXT-1") {
                    ServerLabels::Supported
                } else {
                    ServerLabels::LocalOnly
                };
                // Never inferred from a capability: see ExpungeMeans.
                caps.expunge = ExpungeMeans::Forbidden;
                self.caps = caps.clone();
                Progress::Done(ProtoOutcome::Caps(Box::new(caps)))
            }
            Job::Folders => {
                let mut roles = Vec::new();
                for line in transcript.untagged.iter().map(|u| u.text.as_str()) {
                    if let Some((path, role)) = folder_role(line) {
                        roles.push((path, role));
                    }
                }
                let mut caps = self.caps.clone();
                caps.folders = FolderRoles(roles);
                self.caps = caps.clone();
                Progress::Done(ProtoOutcome::Caps(Box::new(caps)))
            }
            Job::Envelopes { mailbox } | Job::Flags { mailbox } | Job::Listing { mailbox } => {
                // The transcript's untagged responses are handed up as an Ingest shell; turning
                // them into Messages needs ids and blob storage, which live above this crate.
                Progress::Done(ProtoOutcome::Ingested(Box::new(
                    self.empty_ingest(mailbox, UidValidity::Same),
                )))
            }
            Job::Fetch { remote } => {
                let raw = transcript
                    .untagged
                    .iter()
                    .find(|u| u.text.contains("FETCH"))
                    .map(|u| u.text.clone().into_bytes());
                match raw {
                    Some(raw) => Progress::Done(ProtoOutcome::Fetched { remote, raw }),
                    None => Progress::Failed(ProtoError::Malformed(
                        "a fetch completed with no FETCH response".to_owned(),
                    )),
                }
            }
            Job::Applied => Progress::Done(ProtoOutcome::Applied),
            Job::Watching => Progress::Done(ProtoOutcome::Woken),
        }
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }
}

/// A UID set from remote references, or `None` when none of them are IMAP.
fn uid_set(remotes: &[RemoteRef]) -> Option<String> {
    let uids: Vec<String> = remotes
        .iter()
        .filter_map(|r| match r {
            RemoteRef::Imap { uid, .. } => Some(uid.to_string()),
            RemoteRef::Pop { .. } => None,
        })
        .collect();
    (!uids.is_empty()).then(|| uids.join(","))
}

/// The mailbox these references live in.
///
/// One message may be in several, and a `UID STORE` addresses one selected mailbox, so the
/// caller groups by mailbox before calling — this takes the first as the group's mailbox.
fn mailbox_of(remotes: &[RemoteRef], account: AccountId) -> MailboxRef {
    let path = remotes
        .iter()
        .find_map(|r| match r {
            RemoteRef::Imap { mailbox, .. } => Some(mailbox.clone()),
            RemoteRef::Pop { .. } => None,
        })
        .unwrap_or_else(|| "INBOX".to_owned());
    MailboxRef { account, path }
}

/// Gmail's system label for a role.
fn gmail_label(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "\\Inbox",
        MailboxRole::Archive => "\\All",
        MailboxRole::Sent => "\\Sent",
        MailboxRole::Drafts => "\\Draft",
        MailboxRole::Trash => "\\Trash",
        MailboxRole::Spam => "\\Spam",
    }
}

fn quoted_labels(labels: &[String]) -> String {
    labels
        .iter()
        .map(|l| {
            if l.starts_with('\\') {
                l.clone()
            } else {
                format!("\"{}\"", l.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A role from one `LIST` line, using its SPECIAL-USE attributes only.
///
/// Never the name: Gmail localises them, so a German account has
/// `[Google Mail]/Alle Nachrichten` and matching on "All Mail" finds nothing.
fn folder_role(line: &str) -> Option<(String, MailboxRole)> {
    if !line.starts_with("* LIST") {
        return None;
    }
    let role = if line.contains("\\All") {
        MailboxRole::Archive
    } else if line.contains("\\Sent") {
        MailboxRole::Sent
    } else if line.contains("\\Drafts") || line.contains("\\Draft") {
        MailboxRole::Drafts
    } else if line.contains("\\Trash") {
        MailboxRole::Trash
    } else if line.contains("\\Junk") {
        MailboxRole::Spam
    } else if line.trim_end().ends_with("\"INBOX\"") {
        MailboxRole::Inbox
    } else {
        // \Flagged and \Important are virtual views, not mailboxes: syncing them would
        // multiply every starred message under fresh UIDs.
        return None;
    };
    let path = line.rsplit('"').nth(1)?.to_owned();
    Some((mutf7::decode(&path), role))
}

impl std::fmt::Debug for ImapBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapBackend")
            .field("account", &self.account)
            .field("job", &self.job)
            .field("session", &self.session.is_some())
            .finish()
    }
}
