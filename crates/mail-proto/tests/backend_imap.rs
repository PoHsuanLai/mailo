//! `ImapBackend`: the product decisions, checked as bytes on the wire.
//!
//! What matters here is not that a command was sent but *which* — `BODY.PEEK` rather than
//! `BODY`, a label change rather than a delete, no `EXPUNGE` at all.

mod common;

use common::replay;
use mail_domain::*;
use mail_proto::backend::ImapBackend;
use mail_proto::{
    Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Machine, Progress, ProtoOutcome,
};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn caps(labels: ServerLabels, archive: ArchiveMeans) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::ProviderId,
        watch: WatchMode::Idle,
        archive,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 5 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

fn backend(caps: AccountCaps) -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        caps,
        Box::new(|commands: Vec<ImapCommand>| {
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::OAuth {
                        access: "ya29.token".to_owned(),
                        refresh: "1//refresh".to_owned(),
                        expires_at: chrono::DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
                    },
                    sasl: vec![SaslMech::XOauth2],
                },
                commands,
            )
        }),
    )
}

struct Driven {
    backend: ImapBackend,
    op: Option<ProtoOp>,
}

impl Machine for Driven {
    type Out = ProtoOutcome;
    fn start(&mut self) -> Progress<ProtoOutcome> {
        let op = self.op.take().expect("start called twice");
        self.backend.begin(op)
    }
    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        self.backend.feed(ready)
    }
}

fn imap_ref(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 1,
        uid,
    }
}

/// Fetching headers must never mark the message read.
///
/// `BODY.PEEK[HEADER]` and not `BODY[HEADER]`: the peeking form is IMAP's equivalent of POP3's
/// TOP, and the non-peeking one sets \Seen as a side effect of populating a list view.
#[test]
fn fetching_headers_uses_body_peek() {
    let trace = concat!(
        "# SYNTHETIC. The assertion that matters is the C: line: BODY.PEEK, never BODY.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 42 (UID FLAGS BODY.PEEK[HEADER])\n",
        "S: * 1 FETCH (UID 42 FLAGS (\\Seen))\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchHeaders {
            remotes: vec![imap_ref("INBOX", 42)],
        }),
    };
    let outcome = replay(&mut driven, trace).unwrap();
    assert!(
        matches!(outcome, ProtoOutcome::Fetched { .. }),
        "{outcome:?}"
    );
}

/// Archiving on Gmail is a label change. Nothing is deleted, and no EXPUNGE is issued.
#[test]
fn archiving_on_gmail_removes_the_inbox_label() {
    let trace = concat!(
        "# SYNTHETIC. Gmail files by label: adding \\All and removing \\Inbox archives.\n",
        "# There is deliberately no \\Deleted and no EXPUNGE anywhere in this transcript.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 SELECT \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 1] UIDs valid.\n",
        "S: a002 OK [READ-WRITE] SELECT completed\n",
        "C: a003 UID STORE 42 +X-GM-LABELS (\\All)\n",
        "S: a003 OK Success\n",
        "C: a004 UID STORE 42 -X-GM-LABELS (\\Inbox)\n",
        "S: a004 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::SetMailbox {
            remotes: vec![imap_ref("INBOX", 42)],
            role: MailboxRole::Archive,
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Applied
    ));
}

/// Expunging is refused outright, not gated on a capability.
#[test]
fn expunge_is_refused_because_the_setting_cannot_be_read() {
    let mut backend = backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox));
    let outcome = backend.begin(ProtoOp::Expunge {
        remotes: vec![imap_ref("INBOX", 42)],
    });
    // Gmail may be configured to delete permanently and IMAP cannot report that, so there is
    // nothing to gate on — the command does not exist.
    assert!(matches!(outcome, Progress::Failed(_)), "{outcome:?}");
}

/// Without CONDSTORE, or without a trusted modseq, the sweep fetches every flag.
#[test]
fn a_flag_sweep_without_a_modseq_fetches_everything() {
    let trace = concat!(
        "# SYNTHETIC. No modseq means a full flag fetch: slow and correct, which is the right\n",
        "# way round to be wrong. Dovecot 2.0.18 froze HIGHESTMODSEQ while EXISTS climbed.\n",
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 1:* (UID FLAGS)\n",
        "S: * 1 FETCH (UID 42 FLAGS (\\Seen))\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchFlags {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            since_modseq: None,
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Ingested(_)
    ));
}

/// With CONDSTORE and a modseq, the sweep is one CHANGEDSINCE fetch.
#[test]
fn a_flag_sweep_with_a_modseq_uses_changedsince() {
    let trace = concat!(
        "S: * OK Gimap ready\n",
        "C: a001 AUTHENTICATE XOAUTH2 dXNlcj1hZGFAZXhhbXBsZS50ZXN0AWF1dGg9QmVhcmVyIHlhMjkudG9rZW4BAQ==\n",
        "S: a001 OK authenticated\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 3737642)\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven {
        backend: backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox)),
        op: Some(ProtoOp::FetchFlags {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            since_modseq: Some(3_737_642),
        }),
    };
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Ingested(_)
    ));
}

/// A label change on a server without labels is local, and touches no socket.
#[test]
fn labels_on_a_server_without_them_stay_local() {
    let mut backend = backend(caps(ServerLabels::LocalOnly, ArchiveMeans::LocalOnly));
    match backend.begin(ProtoOp::SetLabels {
        remotes: vec![imap_ref("INBOX", 42)],
        add: vec!["work".to_owned()],
        remove: vec![],
    }) {
        Progress::Done(ProtoOutcome::Applied) => {}
        other => panic!("expected an immediate Applied, got {other:?}"),
    }
}

/// Submission belongs to another backend and another connection.
#[test]
fn submission_is_refused_here() {
    let mut backend = backend(caps(ServerLabels::Supported, ArchiveMeans::DropInbox));
    assert!(matches!(
        backend.begin(ProtoOp::Submit {
            draft: DraftId::generate(),
            raw: BlobId::generate(),
        }),
        Progress::Failed(_)
    ));
}
