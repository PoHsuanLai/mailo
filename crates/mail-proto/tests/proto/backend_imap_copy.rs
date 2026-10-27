//! "Copy to…" over IMAP, as bytes on the wire.
//!
//! A copy is `UID COPY` of the UIDs named, from the mailbox they are in, and nothing else: never
//! a `MOVE`, whatever the server offers, and no `\Deleted` or `EXPUNGE` after it. Where folders
//! are labels (Gmail) it is the folder's label added, and the inbox's left alone.

use crate::common;

use common::{Driven, password_backend, replay};
use mail_domain::*;
use mail_proto::{Backend, Moved, Progress, ProtoOutcome};

fn caps(archive: ArchiveMeans) -> AccountCaps {
    AccountCaps {
        archive,
        // A server that can move: a copy still copies.
        move_ext: MoveExt::Supported,
        ..common::caps()
    }
}

fn folders() -> AccountCaps {
    caps(ArchiveMeans::MoveToFolder("Archive".to_owned()))
}

fn at(mailbox: &str, uidvalidity: u32, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity,
        uid,
    }
}

/// The UIDs named, copied from the mailbox they are in, which is only examined: a copy changes
/// nothing there. The server's `COPYUID` (RFC 4315 §3) says where each copy landed.
#[test]
fn copying_sends_uid_copy_of_the_set_and_reads_where_each_copy_landed() {
    let trace = concat!(
        "# SYNTHETIC. RFC 4315 COPYUID in the tagged OK of a UID COPY.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK logged in\n",
        "C: a002 EXAMINE \"INBOX\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID COPY 9,7 \"&UXZO1mWHTvY-\"\n",
        "S: a003 OK [COPYUID 77 7,9 301:302] copied\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        password_backend(folders()),
        ProtoOp::Copy {
            remotes: vec![at("INBOX", 1, 9), at("INBOX", 1, 7)],
            folder: "其他文件".to_owned(),
        },
    );
    assert_eq!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Copied(vec![
            Moved {
                from: at("INBOX", 1, 9),
                to: Some(at("其他文件", 77, 302)),
            },
            Moved {
                from: at("INBOX", 1, 7),
                to: Some(at("其他文件", 77, 301)),
            },
        ])
    );
}

/// A server without UIDPLUS copies and does not say where: each copy is for the next sync of
/// the folder to find.
#[test]
fn a_copy_the_server_does_not_place_is_left_to_the_sync() {
    let trace = concat!(
        "# SYNTHETIC. UID COPY on a server with no UIDPLUS.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK logged in\n",
        "C: a002 EXAMINE \"Projects/2026\"\n",
        "S: a002 OK [READ-ONLY] EXAMINE completed\n",
        "C: a003 UID COPY 4 \"Projects\"\n",
        "S: a003 OK done\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        password_backend(folders()),
        ProtoOp::Copy {
            remotes: vec![at("Projects/2026", 3, 4)],
            folder: "Projects".to_owned(),
        },
    );
    assert_eq!(
        replay(&mut driven, trace).unwrap(),
        ProtoOutcome::Copied(vec![Moved {
            from: at("Projects/2026", 3, 4),
            to: None,
        }])
    );
}

/// Gmail: the folder's label added, and `\Inbox` kept, which is what a copy into a label is.
#[test]
fn copying_on_gmail_adds_the_folders_label_and_keeps_the_inbox() {
    let trace = concat!(
        "# SYNTHETIC. A copy where folders are labels: the label in, nothing out.\n",
        "S: * OK Gimap ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK logged in\n",
        "C: a002 SELECT \"INBOX\"\n",
        "S: a002 OK [READ-WRITE] SELECT completed\n",
        "C: a003 UID STORE 42 +X-GM-LABELS (\"Money/Bills\")\n",
        "S: a003 OK Success\n",
        "DONE\n"
    );
    let mut driven = Driven::new(
        password_backend(AccountCaps {
            labels: ServerLabels::Supported,
            ..caps(ArchiveMeans::DropInbox)
        }),
        ProtoOp::Copy {
            remotes: vec![at("INBOX", 1, 42)],
            folder: "Money/Bills".to_owned(),
        },
    );
    assert_eq!(replay(&mut driven, trace).unwrap(), ProtoOutcome::Applied);
}

/// Nothing is sent for a copy into the folder it is in, which `COPY` would duplicate, nor where
/// the server has no notion of folders.
#[test]
fn a_copy_with_nowhere_new_to_go_sends_nothing() {
    // (case, capabilities, the copy)
    let cases = [
        (
            "into the folder it is in",
            folders(),
            ProtoOp::Copy {
                remotes: vec![at("Projects", 1, 4)],
                folder: "Projects".to_owned(),
            },
        ),
        (
            "into the inbox, spelled another way",
            folders(),
            ProtoOp::Copy {
                remotes: vec![at("INBOX", 1, 4)],
                folder: "Inbox".to_owned(),
            },
        ),
        (
            "a server that files nothing",
            caps(ArchiveMeans::LocalOnly),
            ProtoOp::Copy {
                remotes: vec![at("INBOX", 1, 4)],
                folder: "Projects".to_owned(),
            },
        ),
        (
            "no address to copy from",
            folders(),
            ProtoOp::Copy {
                remotes: Vec::new(),
                folder: "Projects".to_owned(),
            },
        ),
    ];
    for (case, caps, op) in cases {
        let mut backend = password_backend(caps);
        match backend.begin(op) {
            Progress::Done(ProtoOutcome::Applied) => {}
            other => panic!("{case}: {other:?}"),
        }
    }
}
