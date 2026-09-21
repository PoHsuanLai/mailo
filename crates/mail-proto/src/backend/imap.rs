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
    Envelopes { mailbox: MailboxRef },
    Flags { mailbox: MailboxRef },
    Listing { mailbox: MailboxRef },
    Fetch { remotes: Vec<RemoteRef> },
    Applied,
    Watching,
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
        }
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
                self.queue(vec![ImapCommand::List])
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
                // `RFC822.SIZE` is not decoration: the runtime fetches bodies smallest band
                // first, and without a size every message lands in the same band and the order
                // is arrival order again.
                let items = if matches!(self.caps.labels, ServerLabels::Supported) {
                    "(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE \
                     X-GM-MSGID X-GM-THRID X-GM-LABELS)"
                } else {
                    "(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE)"
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
                    ArchiveMeans::MoveToFolder(target) => {
                        let target = target.clone();
                        self.job = Job::Applied;
                        // `MOVE` where the server has it (RFC 6851): atomic, names no flag, and
                        // the only way to actually move a message. It is not the `\Deleted` +
                        // `EXPUNGE` dance in disguise — it is the primitive that dance was
                        // always a poor imitation of, which is why it is safe here while
                        // `ProtoOp::Expunge` stays refused.
                        //
                        // Without it, `COPY` and stop, and the original stays in the source
                        // mailbox. That is not cosmetic: the next survey reports the message as
                        // still in the inbox, server truth wins once the outbox has settled, and
                        // the user's archive quietly comes undone. It is still the right trade —
                        // the alternative is `\Deleted` on a server whose expunge semantics we
                        // cannot read — but it is a known limitation, not a non-issue.
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
                        self.queue(vec![Self::select(&source, false), action])
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
                    Self::select(&mailbox, true),
                    ImapCommand::UidSearch {
                        criteria: "ALL".to_owned(),
                    },
                ])
            }
            ProtoOp::Watch { mailbox } => {
                self.job = Job::Watching;
                self.queue(vec![Self::select(&mailbox, true), ImapCommand::Idle])
            }
            ProtoOp::Expunge { .. } => Progress::Failed(ProtoError::Unsupported(
                "expunging is forbidden: Gmail may be configured to delete permanently, and \
                 that setting cannot be read over IMAP"
                    .to_owned(),
            )),
            ProtoOp::Append { mailbox, raw, role } => {
                // `Append` is not submission and does not belong with it: it uploads a message
                // into a folder over *this* connection, where `Submit` hands one to an entirely
                // different server. Refusing them together is why a draft composed here never
                // reached the Drafts folder on any other device.
                //
                // The bytes are a `BlobId` in the op and `Vec<u8>` on the command, because
                // reading a blob is I/O: the runtime resolves it before calling.
                self.job = Job::Applied;
                let _ = raw;
                let flags = match role {
                    MailboxRole::Drafts => vec!["\\Draft".to_owned(), "\\Seen".to_owned()],
                    // A copy of something already sent is not unread mail waiting for the user.
                    MailboxRole::Sent => vec!["\\Seen".to_owned()],
                    _ => Vec::new(),
                };
                let Some(body) = self.staged.take() else {
                    return Progress::Failed(ProtoError::Malformed(
                        "append was not staged: call stage() with the message bytes".to_owned(),
                    ));
                };
                self.queue(vec![ImapCommand::Append {
                    mailbox: mailbox.path,
                    flags,
                    raw: body,
                }])
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
            Progress::Failed(e) => return Progress::Failed(e),
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
                    gone: Vec::new(),
                })))
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
                    gone: Vec::new(),
                })))
            }
            Job::Fetch { remotes } => {
                let bodies: Vec<Vec<u8>> = transcript
                    .untagged
                    .iter()
                    .filter(|u| u.text.contains("FETCH"))
                    .map(|u| u.text.clone().into_bytes())
                    .collect();
                Progress::Done(ProtoOutcome::Fetched {
                    items: remotes.into_iter().zip(bodies).collect(),
                })
            }
            Job::Applied => Progress::Done(ProtoOutcome::Applied),
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

/// One message as an envelope walk saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Surveyed {
    remote: RemoteRef,
    size: u64,
    read: mail_domain::ReadState,
    star: mail_domain::Star,
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
        let Some(uid) = after_atom(&line.text, "UID ").and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        // Absent size is 0, which sorts into the first band. Fetching a small message early is
        // the cheap mistake; treating an unknown size as enormous would defer it for ever.
        let size = after_atom(&line.text, "RFC822.SIZE ")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let flags = flag_list(&line.text);
        out.push(Surveyed {
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

/// The value following `key`, up to the next space or closing paren.
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
