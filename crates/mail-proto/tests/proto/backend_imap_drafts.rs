//! A draft's copy in the Drafts folder over IMAP, as bytes on the wire.
//!
//! The second place this client sets `\Deleted`, so the traces pin what it may send: a search
//! for the draft's `Message-ID` before anything else, the upload, and then `UID STORE
//! +FLAGS.SILENT (\Deleted)` and `UID EXPUNGE` of exactly the UIDs that search found, in a folder
//! the server lists as Drafts. Without `UIDPLUS` the old copies are marked and left; a bare
//! `EXPUNGE` never appears.
//!
//! SYNTHETIC traces, written from RFC 3501 §6.4.4 (`HEADER`), §6.3.11 (`APPEND`) and RFC 4315
//! §2.1 (`UID EXPUNGE`).

use crate::common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{Driven, password_backend, replay};
use mail_domain::testing::account;
use mail_domain::*;
use mail_proto::backend::ImapBackend;
use mail_proto::{Backend, Progress, ProtoError, ProtoOutcome};

const RAW: &[u8] = b"Subject: hi\r\n\r\nbody\r\n";
const ID: &str = "0b5e7c1a-0000-4000-8000-00000000d1a7@example.test";

fn caps() -> AccountCaps {
    AccountCaps {
        folders: FolderRoles(vec![
            ("Drafts".to_owned(), MailboxRole::Drafts),
            ("Trash".to_owned(), MailboxRole::Trash),
        ]),
        ..common::caps()
    }
}

fn drafts(path: &str) -> MailboxRef {
    MailboxRef {
        account: account(),
        path: path.to_owned(),
    }
}

fn keep() -> Driven<ImapBackend> {
    let mut backend = password_backend(caps());
    backend.stage_append(RAW.to_vec());
    Driven::new(
        backend,
        ProtoOp::KeepDraft {
            draft: DraftId::generate(),
            mailbox: drafts("Drafts"),
            message_id: ID.to_owned(),
        },
    )
}

fn drop_draft() -> Driven<ImapBackend> {
    Driven::new(
        password_backend(caps()),
        ProtoOp::DropDraft {
            draft: DraftId::generate(),
            mailbox: drafts("Drafts"),
            message_id: ID.to_owned(),
        },
    )
}

/// Signed in, asked what the server offers (`capabilities`), Drafts selected, and the search.
fn prelude(capabilities: &str) -> String {
    format!(
        "S: * OK ready\n\
         C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n\
         S: a001 OK logged in\n\
         C: a002 CAPABILITY\n\
         S: * CAPABILITY {capabilities}\n\
         S: a002 OK done\n\
         C: a003 SELECT \"Drafts\"\n\
         S: * 3 EXISTS\n\
         S: * OK [UIDVALIDITY 900] ok\n\
         S: a003 OK [READ-WRITE] done\n\
         C: a004 UID SEARCH DRAFT HEADER Message-ID \"<{ID}>\"\n"
    )
}

/// The upload, as command `tag`: the line, the server's `+`, and the literal.
fn append(tag: &str) -> String {
    let mut literal = RAW.to_vec();
    literal.extend_from_slice(b"\r\n");
    format!(
        "C: {tag} APPEND \"Drafts\" (\\Draft \\Seen) {{21}}\n\
         S: + Ready for literal data\n\
         C64: {}\n\
         S: * 4 EXISTS\n\
         S: {tag} OK APPEND completed\n",
        STANDARD.encode(&literal)
    )
}

#[test]
fn a_first_copy_is_searched_for_then_uploaded_and_nothing_is_marked() {
    let trace = format!(
        "# Nothing carries the draft's Message-ID yet: the upload, and nothing after it.\n\
         {}\
         S: * SEARCH\n\
         S: a004 OK done\n\
         {}\
         DONE\n",
        prelude("IMAP4rev1 UIDPLUS"),
        append("a005"),
    );
    assert_eq!(replay(&mut keep(), &trace).unwrap(), ProtoOutcome::Applied);
}

#[test]
fn the_copies_found_before_the_upload_are_marked_and_expunged_by_uid_after_it() {
    let trace = format!(
        "# Two earlier copies, 4 and 6: one an upload cut off before its answer. Found first,\n\
         # so the new copy is never among them, and removed only once it is there.\n\
         {}\
         S: * SEARCH 4 6\n\
         S: a004 OK done\n\
         {}\
         C: a006 UID STORE 4,6 +FLAGS.SILENT (\\Deleted)\n\
         S: a006 OK Store completed\n\
         C: a007 UID EXPUNGE 4,6\n\
         S: * 3 EXPUNGE\n\
         S: * 2 EXPUNGE\n\
         S: a007 OK Expunge completed\n\
         DONE\n",
        prelude("IMAP4rev1 UIDPLUS"),
        append("a005"),
    );
    assert_eq!(replay(&mut keep(), &trace).unwrap(), ProtoOutcome::Applied);
}

#[test]
fn without_uidplus_the_old_copies_are_marked_and_left_unexpunged() {
    let trace = format!(
        "# No UIDPLUS: only a bare EXPUNGE could remove them, and it would take every other\n\
         # \\Deleted message in Drafts with them. Marked, and the walk ends there.\n\
         {}\
         S: * SEARCH 4\n\
         S: a004 OK done\n\
         {}\
         C: a006 UID STORE 4 +FLAGS.SILENT (\\Deleted)\n\
         S: a006 OK Store completed\n\
         DONE\n",
        prelude("IMAP4rev1"),
        append("a005"),
    );
    assert_eq!(replay(&mut keep(), &trace).unwrap(), ProtoOutcome::Applied);
}

#[test]
fn an_imap4rev2_server_expunges_the_old_copies_without_advertising_uidplus() {
    let trace = format!(
        "{}\
         S: * SEARCH 4\n\
         S: a004 OK done\n\
         {}\
         C: a006 UID STORE 4 +FLAGS.SILENT (\\Deleted)\n\
         S: a006 OK done\n\
         C: a007 UID EXPUNGE 4\n\
         S: * 3 EXPUNGE\n\
         S: a007 OK done\n\
         DONE\n",
        prelude("IMAP4rev1 IMAP4rev2"),
        append("a005"),
    );
    assert_eq!(replay(&mut keep(), &trace).unwrap(), ProtoOutcome::Applied);
}

#[test]
fn dropping_a_draft_marks_and_expunges_every_copy_found_and_uploads_nothing() {
    let trace = format!(
        "# Sent, or discarded: every copy goes.\n\
         {}\
         S: * SEARCH 9\n\
         S: a004 OK done\n\
         C: a005 UID STORE 9 +FLAGS.SILENT (\\Deleted)\n\
         S: a005 OK done\n\
         C: a006 UID EXPUNGE 9\n\
         S: * 3 EXPUNGE\n\
         S: a006 OK done\n\
         DONE\n",
        prelude("IMAP4rev1 UIDPLUS"),
    );
    assert_eq!(
        replay(&mut drop_draft(), &trace).unwrap(),
        ProtoOutcome::Applied
    );
}

#[test]
fn dropping_a_draft_with_no_copy_there_sends_nothing_after_the_search() {
    let trace = format!(
        "{}\
         S: * SEARCH\n\
         S: a004 OK done\n\
         DONE\n",
        prelude("IMAP4rev1 UIDPLUS"),
    );
    assert_eq!(
        replay(&mut drop_draft(), &trace).unwrap(),
        ProtoOutcome::Applied
    );
}

#[test]
fn only_the_search_s_own_answer_names_what_is_marked() {
    let trace = format!(
        "# A SEARCH line during SELECT, which no server sends but a broken one might: not the\n\
         # search's answer, so nothing it names is touched.\n\
         S: * OK ready\n\
         C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n\
         S: a001 OK logged in\n\
         C: a002 CAPABILITY\n\
         S: * CAPABILITY IMAP4rev1 UIDPLUS\n\
         S: a002 OK done\n\
         C: a003 SELECT \"Drafts\"\n\
         S: * SEARCH 1 2 3\n\
         S: * OK [UIDVALIDITY 900] ok\n\
         S: a003 OK [READ-WRITE] done\n\
         C: a004 UID SEARCH DRAFT HEADER Message-ID \"<{ID}>\"\n\
         S: * SEARCH\n\
         S: a004 OK done\n\
         DONE\n",
    );
    assert_eq!(
        replay(&mut drop_draft(), &trace).unwrap(),
        ProtoOutcome::Applied
    );
}

#[test]
fn outside_the_drafts_folder_nothing_is_sent() {
    // (folder, why it is not Drafts)
    const CASES: &[(&str, &str)] = &[
        ("INBOX", "the inbox"),
        ("Trash", "a role, but not Drafts"),
        ("Projects/2026", "a folder of the user's own"),
    ];
    for (folder, why) in CASES {
        for op in [
            ProtoOp::KeepDraft {
                draft: DraftId::generate(),
                mailbox: drafts(folder),
                message_id: ID.to_owned(),
            },
            ProtoOp::DropDraft {
                draft: DraftId::generate(),
                mailbox: drafts(folder),
                message_id: ID.to_owned(),
            },
        ] {
            let mut backend = password_backend(caps());
            backend.stage_append(RAW.to_vec());
            match backend.begin(op) {
                Progress::Failed(ProtoError::Unsupported(text)) => {
                    assert!(text.contains(folder), "{folder} ({why}): {text}")
                }
                other => panic!("{folder} ({why}): {other:?}"),
            }
        }
    }
}

#[test]
fn a_message_id_that_could_break_out_of_the_search_is_refused() {
    for id in [
        "",
        "no-at-sign",
        "a\"@example.test",
        "a@b@example.test",
        "a@example.test\r\nA1 DELETE INBOX",
        "a>@example.test",
    ] {
        let mut backend = password_backend(caps());
        let outcome = backend.begin(ProtoOp::DropDraft {
            draft: DraftId::generate(),
            mailbox: drafts("Drafts"),
            message_id: id.to_owned(),
        });
        assert!(
            matches!(outcome, Progress::Failed(ProtoError::Malformed(_))),
            "{id:?}: {outcome:?}"
        );
    }
}
