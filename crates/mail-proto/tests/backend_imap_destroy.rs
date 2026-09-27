//! "Delete forever" over IMAP, as bytes on the wire.
//!
//! The one place this client sets `\Deleted` and expunges, so the traces pin exactly what it may
//! send: `UID STORE +FLAGS.SILENT (\Deleted)` and `UID EXPUNGE` of the same UIDs (RFC 4315 §2.1),
//! in a folder the server lists as Trash or Spam, and nothing at all where the server lacks
//! `UIDPLUS`, has renumbered the mailbox, or the folder is anything else. A bare `EXPUNGE` never
//! appears: it would remove every `\Deleted` message in the mailbox, other clients' included.

mod common;

use common::replay;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{
    Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Machine, Progress, ProtoError,
    ProtoOutcome,
};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(vec![
            ("Trash".to_owned(), MailboxRole::Trash),
            ("Junk".to_owned(), MailboxRole::Spam),
            ("Sent".to_owned(), MailboxRole::Sent),
        ]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        // The preset default, and it stays so: destroying in Trash is not expunging in general.
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 2 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

fn backend() -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        caps(),
        Box::new(|auth: Authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::Password("hunter2".to_owned()),
                    sasl: vec![SaslMech::Plain],
                },
                all,
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

fn at(mailbox: &str, uidvalidity: u32, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity,
        uid,
    }
}

fn destroy(remotes: Vec<RemoteRef>) -> Driven {
    Driven {
        backend: backend(),
        op: Some(ProtoOp::Destroy { remotes }),
    }
}

#[test]
fn with_uidplus_the_named_uids_are_marked_and_expunged_by_uid() {
    let trace = concat!(
        "# SYNTHETIC, shaped on RFC 4315 §2.1 and RFC 9051 §6.4.9. Trash holds 7 and 9 for this\n",
        "# client and 8, which another client marked \\Deleted: UID EXPUNGE 7,9 leaves 8 alone.\n",
        "S: * OK [CAPABILITY IMAP4rev1] ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 UIDPLUS MOVE SPECIAL-USE\n",
        "S: a002 OK Capability completed\n",
        "C: a003 SELECT \"Trash\"\n",
        "S: * 3 EXISTS\n",
        "S: * OK [UIDVALIDITY 1700] UIDs valid\n",
        "S: * OK [UIDNEXT 10] Predicted next UID\n",
        "S: a003 OK [READ-WRITE] Select completed\n",
        "C: a004 UID STORE 7,9 +FLAGS.SILENT (\\Deleted)\n",
        "S: a004 OK Store completed\n",
        "C: a005 UID EXPUNGE 7,9\n",
        "S: * 3 EXPUNGE\n",
        "S: * 1 EXPUNGE\n",
        "S: a005 OK Expunge completed\n",
        "DONE\n"
    );
    let mut driven = destroy(vec![at("Trash", 1700, 7), at("Trash", 1700, 9)]);
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Applied
    ));
}

#[test]
fn an_imap4rev2_server_has_uid_expunge_without_advertising_uidplus() {
    let trace = concat!(
        "# SYNTHETIC. RFC 9051 §6.4.9 makes UID EXPUNGE part of IMAP4rev2 itself. imap-proto reads\n",
        "# a capability list only with IMAP4rev1 in it, so this server lists both, as dual ones do.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 IMAP4rev2\n",
        "S: a002 OK done\n",
        "C: a003 SELECT \"Junk\"\n",
        "S: * OK [UIDVALIDITY 5] ok\n",
        "S: a003 OK [READ-WRITE] done\n",
        "C: a004 UID STORE 12 +FLAGS.SILENT (\\Deleted)\n",
        "S: a004 OK done\n",
        "C: a005 UID EXPUNGE 12\n",
        "S: * 1 EXPUNGE\n",
        "S: a005 OK done\n",
        "DONE\n"
    );
    let mut driven = destroy(vec![at("Junk", 5, 12)]);
    assert!(matches!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Applied
    ));
}

#[test]
fn without_uidplus_nothing_is_marked_and_the_refusal_says_why() {
    let trace = concat!(
        "# SYNTHETIC. No UIDPLUS: \\Deleted could then only be cleared by a bare EXPUNGE, which\n",
        "# would take every other \\Deleted message in Trash with it. The walk stops after SELECT,\n",
        "# before any STORE, and the trace ending here is the proof nothing else was written.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 MOVE\n",
        "S: a002 OK done\n",
        "C: a003 SELECT \"Trash\"\n",
        "S: * OK [UIDVALIDITY 1700] ok\n",
        "S: a003 OK [READ-WRITE] done\n",
        "FAIL Unsupported\n"
    );
    let mut driven = destroy(vec![at("Trash", 1700, 7)]);
    let err = replay(&mut driven, trace).unwrap_err();
    let ProtoError::Unsupported(why) = &err else {
        panic!("{err:?}");
    };
    assert!(why.contains("UIDPLUS"), "{why}");
    // Fatal, so the outbox stops and tells the user rather than retrying.
    assert!(matches!(err.retry(), Retry::Fatal(_)), "{err:?}");
}

#[test]
fn a_renumbered_mailbox_is_not_expunged() {
    let trace = concat!(
        "# SYNTHETIC. The UIDs were given under UIDVALIDITY 1700; the server now says 1800, so 7\n",
        "# may be someone else's message. Nothing is marked.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 UIDPLUS\n",
        "S: a002 OK done\n",
        "C: a003 SELECT \"Trash\"\n",
        "S: * OK [UIDVALIDITY 1800] ok\n",
        "S: a003 OK [READ-WRITE] done\n",
        "FAIL Refused\n"
    );
    let mut driven = destroy(vec![at("Trash", 1700, 7)]);
    replay(&mut driven, trace).unwrap_err();
}

#[test]
fn outside_trash_and_spam_nothing_is_sent() {
    // (folder, why it is not Trash or Spam)
    const CASES: &[(&str, &str)] = &[
        ("INBOX", "the inbox"),
        ("Sent", "a role, but not one mail is destroyed from"),
        ("Archive", "the archive folder"),
        ("Projects/2026", "a folder of the user's own"),
    ];
    for (folder, why) in CASES {
        let mut backend = backend();
        let outcome = backend.begin(ProtoOp::Destroy {
            remotes: vec![at(folder, 1, 7)],
        });
        match outcome {
            Progress::Failed(ProtoError::Unsupported(text)) => {
                assert!(text.contains(folder), "{folder} ({why}): {text}")
            }
            other => panic!("{folder} ({why}): {other:?}"),
        }
    }
}

#[test]
fn one_deletion_never_spans_two_mailboxes() {
    let mut backend = backend();
    let outcome = backend.begin(ProtoOp::Destroy {
        remotes: vec![at("Trash", 1, 7), at("Junk", 1, 7)],
    });
    assert!(
        matches!(outcome, Progress::Failed(ProtoError::Malformed(_))),
        "{outcome:?}"
    );
}

#[test]
fn expunging_in_general_stays_refused() {
    // Destroy is the exception, not a change of rule: Expunge is still refused, in Trash too.
    let mut backend = backend();
    let outcome = backend.begin(ProtoOp::Expunge {
        remotes: vec![at("Trash", 1, 7)],
    });
    assert!(matches!(outcome, Progress::Failed(_)), "{outcome:?}");
}
