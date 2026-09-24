//! `ProtoOp::Append` on the wire: the literal sent the way the server asked for it, and the
//! address the server gave the message read back out of its completion.
//!
//! SYNTHETIC traces, written from RFC 3501 §6.3.11, RFC 7888 (`LITERAL+`, `LITERAL-`) and
//! RFC 4315 §3 (`APPENDUID`).

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::replay;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{
    Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Machine, Progress, ProtoOutcome,
};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));

const RAW: &[u8] = b"Subject: hi\r\n\r\nbody\r\n";

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
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

fn append() -> Driven {
    let mut backend = backend();
    backend.stage_append(RAW.to_vec());
    Driven {
        backend,
        op: Some(ProtoOp::Append {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: "Archive".to_owned(),
            },
            flags: vec![SystemFlag::Seen, SystemFlag::Flagged],
            date: chrono::DateTime::from_timestamp(1_641_032_430, 0),
            raw: BlobId::generate(),
        }),
    }
}

const PRELUDE: &str = concat!(
    "S: * OK ready\n",
    "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
    "S: a001 OK logged in\n",
    "C: a002 CAPABILITY\n",
);

/// Without `LITERAL+` the bytes wait for the server's `+`; `APPENDUID` says where they went.
#[test]
fn a_synchronizing_literal_waits_for_the_continuation_and_appenduid_is_kept() {
    let mut literal = RAW.to_vec();
    literal.extend_from_slice(b"\r\n");
    let trace = format!(
        "{PRELUDE}\
         S: * CAPABILITY IMAP4rev1 UIDPLUS\n\
         S: a002 OK done\n\
         C: a003 APPEND \"Archive\" (\\Seen \\Flagged) \"01-Jan-2022 10:20:30 +0000\" {{21}}\n\
         S: + Ready for literal data\n\
         C64: {}\n\
         S: a003 OK [APPENDUID 38505 3955] APPEND completed\n\
         DONE\n",
        STANDARD.encode(&literal)
    );
    let outcome = replay(&mut append(), &trace).unwrap();
    assert_eq!(
        outcome,
        ProtoOutcome::Appended {
            remote: Some(RemoteRef::Imap {
                mailbox: "Archive".to_owned(),
                uidvalidity: 38505,
                uid: 3955,
            })
        }
    );
}

/// With `LITERAL+` the line and the literal go in one write, and no `+` is awaited. A server
/// that gives no `APPENDUID` leaves the address unknown rather than guessed.
#[test]
fn a_non_synchronizing_literal_follows_the_line_at_once() {
    let mut write =
        b"a003 APPEND \"Archive\" (\\Seen \\Flagged) \"01-Jan-2022 10:20:30 +0000\" {21+}\r\n"
            .to_vec();
    write.extend_from_slice(RAW);
    write.extend_from_slice(b"\r\n");
    let trace = format!(
        "{PRELUDE}\
         S: * CAPABILITY IMAP4rev1 LITERAL+\n\
         S: a002 OK done\n\
         C64: {}\n\
         S: a003 OK APPEND completed\n\
         DONE\n",
        STANDARD.encode(&write)
    );
    let outcome = replay(&mut append(), &trace).unwrap();
    assert_eq!(outcome, ProtoOutcome::Appended { remote: None });
}

/// `LITERAL-` promises the same only up to 4096 bytes; a small message qualifies.
#[test]
fn literal_minus_covers_a_small_message() {
    let mut write =
        b"a003 APPEND \"Archive\" (\\Seen \\Flagged) \"01-Jan-2022 10:20:30 +0000\" {21+}\r\n"
            .to_vec();
    write.extend_from_slice(RAW);
    write.extend_from_slice(b"\r\n");
    let trace = format!(
        "{PRELUDE}\
         S: * CAPABILITY IMAP4rev1 LITERAL-\n\
         S: a002 OK done\n\
         C64: {}\n\
         S: a003 OK [APPENDUID 7 1] done\n\
         DONE\n",
        STANDARD.encode(&write)
    );
    assert!(matches!(
        replay(&mut append(), &trace).unwrap(),
        ProtoOutcome::Appended { remote: Some(_) }
    ));
}

/// An upload with nothing staged is refused before anything is sent.
#[test]
fn an_append_with_no_bytes_staged_is_refused() {
    let mut driven = append();
    driven.backend = backend();
    assert!(matches!(driven.start(), Progress::Failed(_)));
}
