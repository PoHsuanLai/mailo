//! IMAP as a [`Backend`].
//!
//! Submission is deliberately absent: it is a separate connection to a separate host and lives
//! in [`crate::backend::SmtpBackend`].

use super::folders::{already_so, folder_commands};
use crate::imap::{ImapCommand, ImapSession};
use crate::machine::{Backend, IoReady, Machine, Progress, ProtoError, ProtoOutcome};
use crate::mutf7;
use mail_domain::{
    AccountCaps, AccountId, ArchiveMeans, Condstore, ExpungeMeans, FetchSince, FolderRoles,
    FolderWork, Ingest, MailboxRef, MailboxRole, MoveExt, PartTree, ProtoOp, RemoteRef, Resync,
    ServerLabels, SyncCursor, SystemFlag, UidValidity,
};

/// Builds a session for one command walk, owning the credential so the backend never sees it.
///
/// Takes [`Authenticate`] for the same reason POP3's does: the factory is the only thing holding
/// the credential and therefore the only thing that can know whether this account logs in with a
/// password or a bearer token. This backend used to name `AUTHENTICATE XOAUTH2` itself, at eleven
/// call sites, which meant password IMAP could not work through it at all however well the
/// session supported `LOGIN` — and every non-Gmail server is password IMAP.
pub type SessionFactory =
    Box<dyn FnMut(Authenticate, Vec<ImapCommand>) -> Result<ImapSession, ProtoError> + Send>;

pub use super::Authenticate;

/// What the backend is in the middle of.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Idle,
    Caps,
    Folders,
    Envelopes {
        mailbox: MailboxRef,
    },
    Flags {
        mailbox: MailboxRef,
    },
    Listing {
        mailbox: MailboxRef,
    },
    Resyncing {
        mailbox: MailboxRef,
        since: Resync,
    },
    Fetch {
        remotes: Vec<RemoteRef>,
    },
    Structure {
        remotes: Vec<RemoteRef>,
    },
    Sections {
        remote: RemoteRef,
    },
    Applied,
    /// An upload into `mailbox`, whose completion may say where it landed.
    Appending {
        mailbox: String,
    },
    Watching,
    /// A change to the set of mailboxes, kept so a refusal that says the change has already
    /// happened can be recognised as success.
    Folder(FolderWork),
}

/// Drives [`ImapSession`] on behalf of an account.
pub struct ImapBackend {
    account: AccountId,
    caps: AccountCaps,
    build: SessionFactory,
    session: Option<ImapSession>,
    /// Bytes for the next [`ProtoOp::Append`].
    ///
    /// Staged separately for the same reason `SmtpBackend` stages a submission: the op names a
    /// `BlobId`, and resolving one means reading the blob store, which is above this crate.
    staged: Option<Vec<u8>>,
    /// What the last envelope walk found on the server: every UID, with its size.
    ///
    /// Held because the runtime asks for it through [`Backend::surveyed`] after the walk, for
    /// the same reason POP3 does — on a first sync the store knows nothing, so "what should I
    /// fetch" cannot be answered by asking the store.
    survey: Vec<(RemoteRef, u64)>,
    job: Job,
}

impl ImapBackend {
    pub fn new(account: AccountId, caps: AccountCaps, build: SessionFactory) -> Self {
        Self {
            account,
            caps,
            build,
            session: None,
            staged: None,
            survey: Vec::new(),
            job: Job::Idle,
        }
    }

    pub fn account(&self) -> AccountId {
        self.account
    }

    fn queue(&mut self, commands: Vec<ImapCommand>) -> Progress<ProtoOutcome> {
        self.queue_as(Authenticate::First, commands)
    }

    fn queue_as(
        &mut self,
        auth: Authenticate,
        commands: Vec<ImapCommand>,
    ) -> Progress<ProtoOutcome> {
        let session = match (self.build)(auth, commands) {
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
            qresync: None,
        }
    }

    /// Move `set` out of `source` into `target`.
    ///
    /// `MOVE` where the server has it (RFC 6851): atomic, names no flag, and the only way to
    /// actually move a message. It is not the `\Deleted` + `EXPUNGE` dance in disguise — it is
    /// the primitive that dance was always a poor imitation of, which is why it is safe here
    /// while `ProtoOp::Expunge` stays refused.
    ///
    /// Without it, `COPY` and stop, and the original stays in the source mailbox. That is not
    /// cosmetic: the next survey reports the message as still in the inbox, server truth wins
    /// once the outbox has settled, and the user's archive quietly comes undone. It is still the
    /// right trade — the alternative is `\Deleted` on a server whose expunge semantics we cannot
    /// read — but it is a known limitation, not a non-issue.
    fn move_into(
        &mut self,
        source: &MailboxRef,
        set: String,
        target: String,
    ) -> Progress<ProtoOutcome> {
        self.job = Job::Applied;
        let action = match self.caps.move_ext {
            MoveExt::Supported => ImapCommand::UidMove {
                set,
                mailbox: target,
            },
            MoveExt::Absent => ImapCommand::UidCopy {
                set,
                mailbox: target,
            },
        };
        self.queue(vec![Self::select(source, false), action])
    }

    /// One `UID FETCH` over a batch, on one authenticated connection.
    ///
    /// A batch rather than one message per operation: a connection authenticates once and then
    /// serves many commands, and a fetch per connection would be thousands of them.
    fn fetch(&mut self, remotes: Vec<RemoteRef>, items: &str) -> Progress<ProtoOutcome> {
        let Some(set) = uid_set(&remotes) else {
            return Progress::Failed(ProtoError::Unsupported(
                "a POP reference cannot be fetched over IMAP".to_owned(),
            ));
        };
        let mailbox = mailbox_of(&remotes, self.account);
        self.job = Job::Fetch { remotes };
        self.queue(vec![
            Self::select(&mailbox, true),
            ImapCommand::UidFetch {
                set,
                items: items.to_owned(),
            },
        ])
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
                self.queue(vec![ImapCommand::Capability, ImapCommand::Capability])
            }
            ProtoOp::ListFolders => {
                self.job = Job::Folders;
                // `LSUB` beside `LIST`, because only it says which mailboxes the user follows,
                // and every server has it; `LIST ... RETURN (SUBSCRIBED)` needs LIST-EXTENDED.
                self.queue(vec![ImapCommand::List, ImapCommand::Lsub])
            }
            ProtoOp::Folder(work) => {
                let commands = folder_commands(&work);
                self.job = Job::Folder(work);
                self.queue(commands)
            }
            ProtoOp::FetchEnvelopes { mailbox, since } => {
                let set = match since {
                    FetchSince::Beginning => "1:*".to_owned(),
                    FetchSince::After { cursor } => match cursor {
                        SyncCursor::Imap { uidnext, .. } => format!("{uidnext}:*"),
                        // Another protocol's cursor on an IMAP mailbox is a caller bug, not a
                        // fetch range.
                        SyncCursor::Pop | SyncCursor::Graph { .. } | SyncCursor::Jmap { .. } => {
                            return Progress::Failed(ProtoError::Malformed(
                                "only an IMAP cursor can resume an IMAP mailbox".to_owned(),
                            ));
                        }
                    },
                };
                self.job = Job::Envelopes {
                    mailbox: mailbox.clone(),
                };
                // Exactly what `parse_fetches` reads, and nothing else.
                //
                // `RFC822.SIZE` is not decoration: the runtime fetches bodies smallest band
                // first, and without a size every message lands in the same band and the order
                // is arrival order again. `ENVELOPE`, `BODYSTRUCTURE`, `INTERNALDATE` and the
                // `X-GM-*` set were all asked for and thrown away — the walk keeps a UID, a size
                // and two flags, and headers arrive later from `BODY.PEEK[HEADER]`. Measured
                // against the local fixture over 22 messages, a third of them multipart with an
                // attachment: 10162 bytes asked for against 922 read, eleven times the response
                // for nothing. On the 2372-message maildrop this client is for, that is about a
                // megabyte per walk, every five minutes, on a campus link.
                //
                // Asking for data is also asking a server to *produce* it, which is a second
                // cost and a second risk: F112 is a server that crashes generating a
                // BODYSTRUCTURE we would have discarded.
                //
                // When labels are implemented, `X-GM-LABELS` comes back — together with the code
                // that reads it, which is the only condition under which asking for something is
                // worth the bytes.
                // `X-GM-LABELS` is back, with the code that reads it — which is the condition
                // F113 set for asking a server for anything. `ENVELOPE`, `BODYSTRUCTURE`,
                // `INTERNALDATE` and the other `X-GM-*` attributes stay gone: still unread.
                let items = if matches!(self.caps.labels, ServerLabels::Supported) {
                    "(UID FLAGS RFC822.SIZE X-GM-LABELS)"
                } else {
                    "(UID FLAGS RFC822.SIZE)"
                };
                self.queue(vec![
                    Self::select(&mailbox, true),
                    ImapCommand::UidFetch {
                        set,
                        items: items.to_owned(),
                    },
                ])
            }
            ProtoOp::FetchHeaders { remotes } => {
                // BODY.PEEK, never BODY: the peeking form does not set \Seen. This is IMAP's
                // equivalent of POP3's TOP, and fetching headers with BODY would mark every
                // message read as a side effect of populating a list.
                self.fetch(remotes, "(UID FLAGS BODY.PEEK[HEADER])")
            }
            ProtoOp::FetchBody { remotes } => self.fetch(remotes, "(UID BODY.PEEK[])"),
            ProtoOp::FetchStructure { remotes } => {
                let progress = self.fetch(remotes.clone(), "(UID BODYSTRUCTURE)");
                self.job = Job::Structure { remotes };
                progress
            }
            ProtoOp::FetchSections { remote, sections } => {
                // Each name goes into the command line, so each is checked against the grammar
                // rather than trusted: they come from a stored row, which came from a server.
                if sections.is_empty() || !sections.iter().all(|s| is_section(s)) {
                    return Progress::Failed(ProtoError::Malformed(format!(
                        "not a section list: {sections:?}"
                    )));
                }
                let items = sections
                    .iter()
                    .map(|s| format!("BODY.PEEK[{s}]"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let progress = self.fetch(vec![remote.clone()], &format!("(UID {items})"));
                self.job = Job::Sections { remote };
                progress
            }
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
                let mut commands = vec![Self::select(&mailbox, false)];
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
            ProtoOp::AddKeyword { remotes, keyword } => {
                let Some(set) = uid_set(&remotes) else {
                    return Progress::Done(ProtoOutcome::Applied);
                };
                let mailbox = mailbox_of(&remotes, self.account);
                self.job = Job::Applied;
                // A server whose `PERMANENTFLAGS` has no `\*` may keep the keyword only for
                // the session, or not at all, and still answer OK (RFC 9051 §6.4.6). That is
                // the server's choice to make; the answer is already recorded locally, so the
                // worst case is another client asking its user again.
                self.queue(vec![
                    Self::select(&mailbox, false),
                    ImapCommand::UidStore {
                        set,
                        what: format!("+FLAGS ({})", keyword_atom(keyword)),
                    },
                ])
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
                let mut commands = vec![Self::select(&mailbox, false)];
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
                    ArchiveMeans::MoveToFolder(archive) => {
                        // The folder serving *this* role. It was the archive folder whatever
                        // the role, so trashing a message filed it in Archive and restoring one
                        // moved it from Archive to Archive.
                        let target = match role {
                            MailboxRole::Archive => Some(archive.clone()),
                            MailboxRole::Inbox => Some(
                                self.caps
                                    .folders
                                    .path(MailboxRole::Inbox)
                                    .unwrap_or("INBOX")
                                    .to_owned(),
                            ),
                            other => self.caps.folders.path(other).map(str::to_owned),
                        };
                        let Some(target) = target else {
                            // Moving into a guessed name loses the message if the guess is
                            // wrong; refusing undoes the local move and says why.
                            return Progress::Failed(ProtoError::Unsupported(format!(
                                "filing into {role:?}: this server named no folder for it"
                            )));
                        };
                        self.move_into(&source, set, target)
                    }
                }
            }
            ProtoOp::File { remotes, folder } => {
                let Some(set) = uid_set(&remotes) else {
                    return Progress::Done(ProtoOutcome::Applied);
                };
                let source = mailbox_of(&remotes, self.account);
                match &self.caps.archive {
                    ArchiveMeans::LocalOnly => Progress::Done(ProtoOutcome::Applied),
                    // Gmail: the folder is a label. Added, and the inbox's taken away — which
                    // is what its own web client's "Move to" does.
                    ArchiveMeans::DropInbox => {
                        self.job = Job::Applied;
                        self.queue(vec![
                            Self::select(&source, false),
                            ImapCommand::UidStore {
                                set: set.clone(),
                                what: format!("+X-GM-LABELS ({})", quoted_labels(&[folder])),
                            },
                            ImapCommand::UidStore {
                                set,
                                what: "-X-GM-LABELS (\\Inbox)".to_owned(),
                            },
                        ])
                    }
                    ArchiveMeans::MoveToFolder(_) => self.move_into(&source, set, folder),
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
                    (Some(modseq), condstore) if condstore.changedsince() => {
                        format!("(UID FLAGS) (CHANGEDSINCE {modseq})")
                    }
                    // No modseq, or none we trust: a full flag fetch is slow and correct, which
                    // is the right way round to be wrong. Dovecot 2.0.18 froze HIGHESTMODSEQ at
                    // 1 while EXISTS climbed, and trusting that means never seeing another
                    // flag change.
                    _ => "(UID FLAGS)".to_owned(),
                };
                self.queue(vec![
                    Self::select(&mailbox, true),
                    ImapCommand::UidFetch {
                        set: "1:*".to_owned(),
                        items,
                    },
                ])
            }
            ProtoOp::ListRemote {
                mailbox,
                since: Some(since),
            } if self.caps.condstore == Condstore::Qresync => {
                // The server says what vanished, so nothing needs listing. `ENABLE` first and
                // on this connection: QRESYNC is per session, and a `SELECT` carrying the
                // parameter without it is a protocol error.
                self.job = Job::Resyncing {
                    mailbox: mailbox.clone(),
                    since,
                };
                self.queue(vec![
                    ImapCommand::Enable("QRESYNC".to_owned()),
                    ImapCommand::Select {
                        mailbox: mailbox.path.clone(),
                        read_only: true,
                        qresync: Some(since),
                    },
                ])
            }
            ProtoOp::ListRemote { mailbox, .. } => {
                self.job = Job::Listing {
                    mailbox: mailbox.clone(),
                };
                // Without QRESYNC — which Gmail does not offer — this is the only way to find
                // what was expunged elsewhere. RFC 7162 says so outright.
                self.queue(vec![
                    Self::select(&mailbox, true),
                    ImapCommand::UidSearch {
                        criteria: "ALL".to_owned(),
                    },
                ])
            }
            ProtoOp::Watch { mailbox, uidnext } => {
                self.job = Job::Watching;
                let idle = match uidnext {
                    Some(uidnext) => ImapCommand::IdleAfter { uidnext },
                    None => ImapCommand::Idle,
                };
                self.queue(vec![Self::select(&mailbox, true), idle])
            }
            ProtoOp::Expunge { .. } => Progress::Failed(ProtoError::Unsupported(
                "expunging is forbidden: Gmail may be configured to delete permanently, and \
                 that setting cannot be read over IMAP"
                    .to_owned(),
            )),
            ProtoOp::Append {
                mailbox,
                flags,
                date,
                raw,
            } => {
                // `Append` is not submission and does not belong with it: it uploads a message
                // into a folder over *this* connection, where `Submit` hands one to an entirely
                // different server. Refusing them together is why a draft composed here never
                // reached the Drafts folder on any other device.
                //
                // The bytes are a `BlobId` in the op and `Vec<u8>` on the command, because
                // reading a blob is I/O: the runtime resolves it before calling.
                let _ = raw;
                let Some(body) = self.staged.take() else {
                    return Progress::Failed(ProtoError::Malformed(
                        "append was not staged: call stage() with the message bytes".to_owned(),
                    ));
                };
                self.job = Job::Appending {
                    mailbox: mailbox.path.clone(),
                };
                // `CAPABILITY` first, after signing in, so the session knows whether the
                // server takes the literal without a `+` (`LITERAL+`). The list before sign-in
                // is often a subset, so the greeting's cannot be trusted for it.
                self.queue(vec![
                    ImapCommand::Capability,
                    ImapCommand::Append {
                        mailbox: mailbox.path,
                        flags: flags.iter().map(|f| wire_flag(*f).to_owned()).collect(),
                        date,
                        raw: body,
                    },
                ])
            }
            ProtoOp::Submit { .. } => Progress::Failed(ProtoError::Unsupported(
                "submission is a separate backend".to_owned(),
            )),
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
            Progress::Failed(e) => {
                return match &self.job {
                    // The server refused because what was asked is already true: the folder
                    // exists, or is already gone. That is the outcome the user wanted, and
                    // undoing it here would leave this client disagreeing with the server.
                    Job::Folder(work) if already_so(work, &e) => {
                        self.job = Job::Idle;
                        Progress::Done(ProtoOutcome::Applied)
                    }
                    _ => Progress::Failed(e),
                };
            }
            Progress::Done(transcript) => transcript,
        };

        match std::mem::replace(&mut self.job, Job::Idle) {
            Job::Idle => Progress::Failed(ProtoError::Malformed(
                "a transcript arrived with no operation in flight".to_owned(),
            )),
            Job::Caps => {
                let mut caps = self.caps.clone();
                // Whole atoms, not substrings: `contains("MOVE")` is also true of `REMOVE`
                // and `contains("UID")` of `UIDPLUS`. See `imap::has_capability`.
                let seen = |name: &str| crate::imap::has_capability(&transcript.capabilities, name);
                // QRESYNC implies CONDSTORE (RFC 7162 §3.2.3), but a server listing one without
                // the other is misconfigured, and the lesser claim is the safe one to believe.
                caps.condstore = match (seen("CONDSTORE"), seen("QRESYNC")) {
                    (true, true) => Condstore::Qresync,
                    (true, false) => Condstore::Supported,
                    (false, _) => Condstore::Absent,
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
                Progress::Done(ProtoOutcome::Folders {
                    caps: Box::new(caps),
                    listed: crate::backend::folders::listing(&transcript.untagged, self.account),
                })
            }
            Job::Listing { mailbox } => {
                // `UID SEARCH ALL` answers with `* SEARCH 101 102 …`, not with FETCH lines, so
                // this cannot share the envelope arm — which is exactly what it used to do, and
                // why `parse_fetches` found nothing and every expunge sweep concluded that
                // nothing had disappeared.
                //
                // What exists is all this can say. Which of those we *hold* is the store's
                // knowledge, so the runtime does the diff and fills in `gone`.
                let (uidvalidity, uidnext, modseq) = mailbox_state(&transcript.untagged);
                self.survey = parse_search(&transcript.untagged, &mailbox.path, uidvalidity)
                    .into_iter()
                    .map(|remote| (remote, 0))
                    .collect();
                Progress::Done(ProtoOutcome::Ingested(Box::new(Ingest {
                    mailbox,
                    validity: UidValidity::Same,
                    cursor: Some(SyncCursor::Imap {
                        uidvalidity,
                        uidnext,
                        modseq,
                    }),
                    messages: Vec::new(),
                    flags: Vec::new(),
                    labels: Vec::new(),
                    label_names: Vec::new(),
                    gone: Vec::new(),
                })))
            }
            Job::Resyncing { mailbox, since } => {
                let (uidvalidity, uidnext, modseq) = mailbox_state(&transcript.untagged);
                let nothing = Ingest {
                    mailbox: mailbox.clone(),
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: Vec::new(),
                    flags: Vec::new(),
                    labels: Vec::new(),
                    label_names: Vec::new(),
                    gone: Vec::new(),
                };
                // A different UIDVALIDITY and the server ignores the parameter (RFC 7162
                // §3.2.5.2): the mailbox was renumbered, nothing here describes the UIDs held,
                // and the sync pass is what notices a renumbering. Report nothing and leave the
                // cursor where it was, so the next sweep does not resume from a mailbox that no
                // longer exists.
                if uidvalidity != since.uidvalidity {
                    return Progress::Done(ProtoOutcome::Resynced {
                        ingest: Box::new(nothing),
                        vanished: Vec::new(),
                    });
                }
                let changed = parse_fetches(&transcript.untagged, &mailbox.path, uidvalidity);
                Progress::Done(ProtoOutcome::Resynced {
                    ingest: Box::new(Ingest {
                        cursor: Some(SyncCursor::Imap {
                            uidvalidity,
                            uidnext,
                            modseq,
                        }),
                        flags: changed
                            .into_iter()
                            .map(|row| (row.remote, row.read, row.star))
                            .collect(),
                        ..nothing
                    }),
                    vanished: parse_vanished(&transcript.untagged),
                })
            }
            Job::Envelopes { mailbox } | Job::Flags { mailbox } => {
                // No `Fetched` here, and that is not laziness: a `Fetched` needs the raw bytes,
                // and an envelope walk deliberately does not fetch them. What this walk learns
                // is what *exists* — every UID, its size and its flags — plus where to resume.
                // Discarding that, which is what this did until an end-to-end test asked an
                // IMAP server for its mail and got an empty mailbox back, leaves the runtime
                // with nothing to fetch and no cursor to fetch it from.
                let mailbox_path = mailbox.path.clone();
                // The mailbox's state first: every reference below is stamped with the
                // UIDVALIDITY it belongs to, because a UID without one names nothing.
                let (uidvalidity, uidnext, modseq) = mailbox_state(&transcript.untagged);
                let seen = parse_fetches(&transcript.untagged, &mailbox_path, uidvalidity);
                self.survey = seen
                    .iter()
                    .map(|row| (row.remote.clone(), row.size))
                    .collect();

                Progress::Done(ProtoOutcome::Ingested(Box::new(Ingest {
                    mailbox,
                    validity: UidValidity::Same,
                    // A survey, so it does move the cursor.
                    cursor: Some(SyncCursor::Imap {
                        uidvalidity,
                        uidnext,
                        modseq,
                    }),
                    messages: Vec::new(),
                    flags: seen
                        .iter()
                        .map(|row| (row.remote.clone(), row.read, row.star))
                        .collect(),
                    labels: Vec::new(),
                    // Only where the server has them. Everywhere else this is empty and the
                    // store does nothing, which is what "labels are local" means on POP3.
                    label_names: seen
                        .iter()
                        .filter(|row| !row.labels.is_empty())
                        .map(|row| (row.remote.clone(), row.labels.clone()))
                        .collect(),
                    gone: Vec::new(),
                })))
            }
            Job::Fetch { remotes } => {
                // The literal's bytes, not the response's text. Taking the text appended the
                // `)` that closes the FETCH to every message and prepended `* n FETCH (...)`,
                // which a lenient MIME parser accepts in silence; and it arrived through
                // `from_utf8_lossy`, so every 8-bit byte in a message became U+FFFD.
                //
                // A `FETCH` with no literal is skipped rather than guessed at: an untagged
                // response that is not carrying a body has no body to offer.
                //
                // Paired by the UID each response carries, never by position. A server answers
                // a UID FETCH in mailbox order whatever order the set was written in, and skips
                // a UID that no longer exists; zipping the request with the replies gave every
                // message after the first mismatch someone else's headers and body, and moved
                // its `remote_map` row onto that other message.
                let mut bodies: std::collections::HashMap<u32, Vec<u8>> = transcript
                    .untagged
                    .iter()
                    .filter(|u| u.text.contains("FETCH"))
                    .filter_map(|u| {
                        let uid = after_atom(&protocol_text(u), "UID ")?.parse().ok()?;
                        Some((uid, u.literal()?.to_vec()))
                    })
                    .collect();
                // The same parser the survey uses. The header fetch asks for `FLAGS`, so they
                // are already on the wire; not reading them here is what left every message
                // unread until a later sweep happened to revisit it.
                let flags = parse_fetches(
                    &transcript.untagged,
                    &mailbox_of(&remotes, self.account).path,
                    uidvalidity_of(&remotes),
                )
                .into_iter()
                .map(|row| (row.remote, row.read, row.star))
                .collect();
                Progress::Done(ProtoOutcome::Fetched {
                    items: remotes
                        .into_iter()
                        .filter_map(|remote| {
                            let body = bodies.remove(&uid_of(&remote)?)?;
                            Some((remote, body))
                        })
                        .collect(),
                    flags,
                })
            }
            Job::Structure { remotes } => {
                let mut out = Vec::new();
                for (uid, attrs) in typed_fetches(&transcript.untagged) {
                    let Some(remote) = remotes.iter().find(|r| uid_of(r) == Some(uid)) else {
                        continue;
                    };
                    let tree = attrs.iter().find_map(|a| match a {
                        imap_proto::AttributeValue::BodyStructure(body) => part_tree(body, &[]),
                        _ => None,
                    });
                    if let Some(tree) = tree {
                        out.push((remote.clone(), tree));
                    }
                }
                Progress::Done(ProtoOutcome::Structures(out))
            }
            Job::Sections { remote } => {
                let mut parts = Vec::new();
                for (_, attrs) in typed_fetches(&transcript.untagged) {
                    for attr in attrs {
                        if let imap_proto::AttributeValue::BodySection {
                            section: Some(path),
                            data,
                            ..
                        } = attr
                        {
                            parts.push((
                                section_name(&path),
                                data.map(|d| d.into_owned()).unwrap_or_default(),
                            ));
                        }
                    }
                }
                Progress::Done(ProtoOutcome::Sections { remote, parts })
            }
            Job::Appending { mailbox } => Progress::Done(ProtoOutcome::Appended {
                remote: transcript
                    .completed
                    .iter()
                    .find_map(|done| appenduid(&done.text))
                    .map(|(uidvalidity, uid)| RemoteRef::Imap {
                        mailbox,
                        uidvalidity,
                        uid,
                    }),
            }),
            Job::Applied | Job::Folder(_) => Progress::Done(ProtoOutcome::Applied),
            Job::Watching => Progress::Done(ProtoOutcome::Woken),
        }
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }

    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        self.survey.clone()
    }

    fn stage_append(&mut self, raw: Vec<u8>) {
        self.staged = Some(raw);
    }
}

/// The wire spelling of a keyword. `$`-prefixed keywords are the registered ones (RFC 5788).
fn keyword_atom(keyword: mail_domain::Keyword) -> &'static str {
    match keyword {
        mail_domain::Keyword::MdnSent => "$MDNSent",
    }
}

/// One message as an envelope walk saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Surveyed {
    remote: RemoteRef,
    size: u64,
    read: mail_domain::ReadState,
    star: mail_domain::Star,
    /// Gmail's user labels, empty everywhere else. See [`user_labels`].
    labels: Vec<String>,
}

/// The UIDVALIDITY a batch of references carries, so re-parsed rows key to the same rows.
///
/// Every reference in one batch comes from one mailbox — a fetch is issued after a `SELECT` —
/// so taking it from the first is exact rather than an approximation.
fn uidvalidity_of(remotes: &[RemoteRef]) -> u32 {
    remotes
        .iter()
        .find_map(|r| match r {
            RemoteRef::Imap { uidvalidity, .. } => Some(*uidvalidity),
            _ => None,
        })
        .unwrap_or(0)
}

/// The user labels in an `X-GM-LABELS (…)` list.
///
/// Gmail mixes two things in that list: labels the user made, and its own names for mailboxes
/// and flags — `\Inbox`, `\Sent`, `\Draft`, `\Trash`, `\Spam`, `\Important`, `\Starred`,
/// `\Muted`. The second kind is already `mailbox` and `read` and `star` here, so carrying them
/// through as labels would put `\Inbox` on every message in the inbox and `\Starred` on
/// everything starred — noise on every row, and a "label" the user cannot remove.
///
/// Anything beginning with a backslash is dropped rather than a fixed list being matched: Gmail
/// has added to that set before, and a client that enumerates it inherits the next name as a
/// label. Real labels cannot start with one — Gmail rejects the character.
///
/// Values are atoms or quoted strings, and a quoted one may contain an escaped quote or
/// backslash. `&` is Gmail's modified UTF-7 for non-ASCII names, which [`crate::mutf7`] decodes:
/// a Chinese label arrives as `&Ux1Tgg-` on the wire and must not be shown that way.
fn user_labels(line: &str) -> Vec<String> {
    let Some(rest) = after_atom_list(line, "X-GM-LABELS ") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut value = String::new();
        if c == '"' {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.push(chars.next().unwrap_or('\\')),
                    '"' => break,
                    other => value.push(other),
                }
            }
        } else {
            value.push(c);
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                value.push(next);
                chars.next();
            }
        }
        if value.is_empty() || value.starts_with('\\') {
            continue;
        }
        out.push(crate::mutf7::decode(&value));
    }
    out
}

/// The contents of the parenthesised list following `key`, if there is one.
///
/// Nesting is not a possibility here — a label list is flat — so counting depth would be
/// answering a question the grammar does not ask.
fn after_atom_list<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let at = line.find(key)? + key.len();
    let rest = line.get(at..)?.strip_prefix('(')?;
    let end = rest.find(')')?;
    rest.get(..end)
}

/// Pull `UID`, `RFC822.SIZE` and `FLAGS` out of untagged `FETCH` responses.
///
/// Text scanning rather than a typed tree, matching what the rest of this backend does with
/// untagged responses and for the reason given on [`crate::Untagged`]: the caller needs shapes
/// this crate has no opinion about, and a lossy translation in the middle is worse than
/// re-reading at the edge.
///
/// A response with no `UID` is skipped rather than guessed at. Sequence numbers shift when
/// anything is expunged, so a message addressed by one is a message addressed wrongly.
fn parse_fetches(untagged: &[crate::Untagged], mailbox: &str, uidvalidity: u32) -> Vec<Surveyed> {
    let mut out = Vec::new();
    for line in untagged.iter().filter(|u| u.text.contains("FETCH")) {
        let text = protocol_text(line);
        let Some(uid) = after_atom(&text, "UID ").and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        // Absent size is 0, which sorts into the first band. Fetching a small message early is
        // the cheap mistake; treating an unknown size as enormous would defer it for ever.
        let size = after_atom(&text, "RFC822.SIZE ")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let flags = flag_list(&text);
        out.push(Surveyed {
            labels: user_labels(&text),
            remote: RemoteRef::Imap {
                mailbox: mailbox.to_owned(),
                // The mailbox's own UIDVALIDITY, not zero. `remote_map` keys on it, so a
                // reference carrying the wrong one is a row that matches no real message — and
                // a reference carrying zero is every mailbox's row at once.
                uidvalidity,
                uid,
            },
            size,
            read: if flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")) {
                mail_domain::ReadState::Read
            } else {
                mail_domain::ReadState::Unread
            },
            star: if flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged")) {
                mail_domain::Star::Starred
            } else {
                mail_domain::Star::Unstarred
            },
        });
    }
    out
}

/// The UIDs in an untagged `SEARCH` response.
///
/// `* SEARCH 101 102 103`, and `* SEARCH` alone for an empty mailbox — which is a real answer
/// meaning "everything you hold is gone", not a parse failure, and must come back as an empty
/// list rather than nothing at all.
fn parse_search(untagged: &[crate::Untagged], mailbox: &str, uidvalidity: u32) -> Vec<RemoteRef> {
    untagged
        .iter()
        .filter_map(|u| {
            let rest = u.text.trim().strip_prefix("* SEARCH")?;
            Some(
                rest.split_whitespace()
                    .filter_map(|n| n.parse::<u32>().ok()),
            )
        })
        .flatten()
        .map(|uid| RemoteRef::Imap {
            mailbox: mailbox.to_owned(),
            uidvalidity,
            uid,
        })
        .collect()
}

/// Every untagged `FETCH`, parsed properly, as `(uid, attributes)`.
///
/// The text-scanning helpers below are enough for flags and sizes. They are not enough for a
/// `BODYSTRUCTURE`, which nests, quotes and carries literals, or for several `BODY[...]`
/// literals in one response, so these go through `imap-proto` from the response's own bytes.
/// A response it cannot parse is skipped: the message it described is fetched whole instead.
fn typed_fetches(
    untagged: &[crate::Untagged],
) -> Vec<(u32, Vec<imap_proto::AttributeValue<'static>>)> {
    untagged
        .iter()
        .filter_map(|u| match imap_proto::parser::parse_response(&u.raw) {
            Ok((_, imap_proto::Response::Fetch(_, attrs))) => {
                let uid = attrs.iter().find_map(|a| match a {
                    imap_proto::AttributeValue::Uid(uid) => Some(*uid),
                    _ => None,
                })?;
                Some((uid, attrs.into_iter().map(|a| a.into_owned()).collect()))
            }
            _ => None,
        })
        .collect()
}

fn uid_of(remote: &RemoteRef) -> Option<u32> {
    match remote {
        RemoteRef::Imap { uid, .. } => Some(*uid),
        RemoteRef::Pop { .. } | RemoteRef::Graph { .. } | RemoteRef::Jmap { .. } => None,
    }
}

/// A `BODYSTRUCTURE` as a [`PartTree`], or `None` if it cannot be rebuilt from its parts.
///
/// `path` is the section of `body` as numbers. The root of a multipart has the empty section;
/// a root that is not multipart is section `1` (RFC 3501 §6.4.5).
fn part_tree(body: &imap_proto::BodyStructure<'_>, path: &[u32]) -> Option<PartTree> {
    use imap_proto::BodyStructure as B;
    let section = |path: &[u32]| {
        if path.is_empty() {
            "1".to_owned()
        } else {
            path.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(".")
        }
    };
    match body {
        B::Multipart { common, bodies, .. } => {
            // Without its boundary a multipart cannot be written back out, and one that is
            // unusable makes the whole tree so.
            let boundary = common
                .ty
                .params
                .iter()
                .flatten()
                .find(|(k, _)| k.eq_ignore_ascii_case("boundary"))
                .map(|(_, v)| v.to_string())?;
            let parts = bodies
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let mut child_path = path.to_vec();
                    child_path.push(i as u32 + 1);
                    part_tree(child, &child_path)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(PartTree::Multipart {
                section: if path.is_empty() {
                    String::new()
                } else {
                    section(path)
                },
                subtype: common.ty.subtype.to_ascii_lowercase(),
                boundary,
                parts,
            })
        }
        B::Basic { common, other, .. }
        | B::Text { common, other, .. }
        | B::Message { common, other, .. } => Some(PartTree::Leaf {
            section: section(path),
            mime: format!("{}/{}", common.ty.ty, common.ty.subtype).to_ascii_lowercase(),
            octets: u64::from(other.octets),
            attachment: common
                .disposition
                .as_ref()
                .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment")),
        }),
    }
}

/// Whether `s` is a section this client asks for: `HEADER`, or dotted part numbers with an
/// optional `.MIME`. Nothing else may reach the command line.
fn is_section(s: &str) -> bool {
    if s == "HEADER" {
        return true;
    }
    let numbers = s.strip_suffix(".MIME").unwrap_or(s);
    !numbers.is_empty()
        && numbers.split('.').all(|n| {
            !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) && n != "0"
        })
}

/// The name a section was asked for by, from the name the server answered with.
fn section_name(path: &imap_proto::SectionPath) -> String {
    use imap_proto::{MessageSection as M, SectionPath as P};
    let word = |m: &M| match m {
        M::Header => "HEADER",
        M::Mime => "MIME",
        M::Text => "TEXT",
    };
    match path {
        P::Full(m) => word(m).to_owned(),
        P::Part(numbers, rest) => {
            let mut name = numbers
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(".");
            if let Some(m) = rest {
                name.push('.');
                name.push_str(word(m));
            }
            name
        }
    }
}

/// UID ranges from every `VANISHED` response, `(EARLIER)` or not, as `(first, last)`.
///
/// A range the server wrote backwards, `9:3`, is the same range (RFC 3501 §9 `seq-range`). A
/// malformed response never gets here — `imap-proto` rejects it and the session fails — but an
/// element that does not parse is skipped rather than guessed at all the same: this list can
/// only ever remove held mail, and the error to make is the one that removes less.
fn parse_vanished(untagged: &[crate::Untagged]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for line in untagged {
        let Some(rest) = line.text.trim().strip_prefix("* VANISHED") else {
            continue;
        };
        let rest = rest.trim_start();
        let set = rest.strip_prefix("(EARLIER)").unwrap_or(rest).trim();
        for element in set.split(',') {
            let (lo, hi) = element.split_once(':').unwrap_or((element, element));
            if let (Ok(lo), Ok(hi)) = (lo.trim().parse::<u32>(), hi.trim().parse::<u32>()) {
                out.push((lo.min(hi), lo.max(hi)));
            }
        }
    }
    out
}

/// The value following `key`, up to the next space or closing paren.
/// A response's text with its literal cut out: the protocol, without the mail.
///
/// A header or body can say `UID 7` or `FLAGS (\Seen)` as easily as the response around it,
/// and a server may put `UID` after the literal as well as before it. Scanning the whole text
/// finds whichever comes first.
fn protocol_text(u: &crate::Untagged) -> std::borrow::Cow<'_, str> {
    let Some(literal) = u.literal() else {
        return std::borrow::Cow::Borrowed(&u.text);
    };
    // `literal` borrows from `raw`, so its offset there is the distance between the two.
    let start = literal.as_ptr() as usize - u.raw.as_ptr() as usize;
    let mut outside = u.raw[..start].to_vec();
    outside.extend_from_slice(&u.raw[start + literal.len()..]);
    std::borrow::Cow::Owned(String::from_utf8_lossy(&outside).into_owned())
}

fn after_atom(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)? + key.len();
    let rest = &text[at..];
    let end = rest.find([' ', ')', '\r', '\n']).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// The atoms inside `FLAGS (...)`.
fn flag_list(text: &str) -> Vec<String> {
    let Some(at) = text.find("FLAGS (") else {
        return Vec::new();
    };
    let rest = &text[at + "FLAGS (".len()..];
    let Some(end) = rest.find(')') else {
        return Vec::new();
    };
    rest[..end].split_whitespace().map(str::to_owned).collect()
}

/// `UIDVALIDITY`, `UIDNEXT` and `HIGHESTMODSEQ` from the responses to `SELECT`/`EXAMINE`.
///
/// Zero and `None` when absent. A server that does not say is a server we cannot resume
/// against, and the next pass surveys from the beginning — correct and slow, rather than wrong
/// and fast.
///
/// `HIGHESTMODSEQ` is `None` rather than zero when missing, because zero is a value a server
/// could legitimately report and "did not say" is not a modseq.
fn mailbox_state(untagged: &[crate::Untagged]) -> (u32, u32, Option<u64>) {
    let find = |key: &str| -> Option<u64> {
        untagged
            .iter()
            .filter_map(|u| {
                let at = u.text.find(key)? + key.len();
                let rest = &u.text[at..];
                let end = rest.find([']', ' ']).unwrap_or(rest.len());
                rest[..end].trim().parse::<u64>().ok()
            })
            .next_back()
    };
    (
        find("UIDVALIDITY ").unwrap_or(0) as u32,
        find("UIDNEXT ").unwrap_or(0) as u32,
        find("HIGHESTMODSEQ "),
    )
}

/// A UID set from remote references, or `None` when none of them are IMAP.
fn uid_set(remotes: &[RemoteRef]) -> Option<String> {
    let uids: Vec<String> = remotes
        .iter()
        .filter_map(|r| match r {
            RemoteRef::Imap { uid, .. } => Some(uid.to_string()),
            RemoteRef::Pop { .. } | RemoteRef::Graph { .. } | RemoteRef::Jmap { .. } => None,
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
            RemoteRef::Pop { .. } | RemoteRef::Graph { .. } | RemoteRef::Jmap { .. } => None,
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
/// A system flag as IMAP spells it.
fn wire_flag(flag: SystemFlag) -> &'static str {
    match flag {
        SystemFlag::Seen => "\\Seen",
        SystemFlag::Answered => "\\Answered",
        SystemFlag::Flagged => "\\Flagged",
        SystemFlag::Draft => "\\Draft",
    }
}

/// `[APPENDUID <uidvalidity> <uid>]` in a tagged `OK` (RFC 4315 §3), as numbers.
///
/// Read from the response code, never assumed from `UIDPLUS` being advertised: servers exist
/// that advertise it and omit the code. A uid *set* — which only a multi-message append
/// returns — is not one message's address and yields nothing.
fn appenduid(text: &str) -> Option<(u32, u32)> {
    let upper = text.to_ascii_uppercase();
    let at = upper.find("[APPENDUID ")?;
    let rest = &text[at + "[APPENDUID ".len()..];
    let inner = &rest[..rest.find(']')?];
    let mut words = inner.split_whitespace();
    let uidvalidity = words.next()?.parse().ok()?;
    let uid = words.next()?.parse().ok()?;
    Some((uidvalidity, uid))
}

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
