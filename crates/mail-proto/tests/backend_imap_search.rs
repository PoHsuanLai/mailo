//! Searching the server over IMAP, as bytes on the wire: `CAPABILITY`, then `EXAMINE` and
//! `UID SEARCH` per mailbox on one connection, read-only, answered by `SEARCH` (RFC 3501 §7.2.5)
//! or, where the server offers it, `ESEARCH` (RFC 4731 §3.1).

mod common;

use common::replay;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_proto::backend::{Authenticate, Found, ImapBackend};
use mail_proto::search::imap::{ImapPlan, SearchKey};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, ProtoError};
use porter_core::SecretText;
use porter_core::{AccountId, Credential};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(vec![("Archive".to_owned(), MailboxRole::Archive)]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 2 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

fn backend() -> ImapBackend {
    ImapBackend::new(
        acct_account(),
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
                    credential: Credential::Password(SecretText::new("hunter2".to_owned())),
                    sasl: vec![SaslMech::Plain],
                },
                all,
            )
        }),
    )
}

fn plan(mailboxes: &[&str], keys: Vec<SearchKey>) -> ImapPlan {
    ImapPlan {
        mailboxes: mailboxes.iter().map(|m| (*m).to_owned()).collect(),
        keys,
    }
}

fn found(mailbox: &str, uidvalidity: u32, uids: &[u32], count: u64) -> Found {
    Found {
        mailbox: mailbox.to_owned(),
        uidvalidity,
        uids: uids.to_vec(),
        count,
    }
}

#[test]
fn without_esearch_each_mailbox_answers_with_its_uids() {
    let trace = concat!(
        "# SYNTHETIC, shaped on RFC 3501 §6.4.4 and §7.2.5. Two mailboxes on one connection,\n",
        "# each examined read-only; the second answer is split across reads.\n",
        "S: * OK [CAPABILITY IMAP4rev1] ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 MOVE SPECIAL-USE\n",
        "S: a002 OK done\n",
        "C: a003 EXAMINE \"INBOX\"\n",
        "S: * 40 EXISTS\n",
        "S: * OK [UIDVALIDITY 1700] UIDs valid\n",
        "S: a003 OK [READ-ONLY] done\n",
        "C: a004 UID SEARCH FROM \"ada\"\n",
        "S: * SEARCH 12 31\n",
        "S: a004 OK Search completed\n",
        "C: a005 EXAMINE \"Archive\"\n",
        "S: * OK [UIDVALIDITY 42] UIDs valid\n",
        "S: a005 OK [READ-ONLY] done\n",
        "C: a006 UID SEARCH FROM \"ada\"\n",
        "SPLIT\n",
        "S: * SEARCH 3 7 5\n",
        "S: a006 OK Search completed\n",
        "DONE\n"
    );
    let mut backend = backend();
    let mut walk = backend.searching(plan(
        &["INBOX", "Archive"],
        vec![SearchKey::From("ada".to_owned())],
    ));
    assert_eq!(
        replay(&mut walk, trace).unwrap(),
        vec![
            found("INBOX", 1700, &[12, 31], 2),
            found("Archive", 42, &[3, 5, 7], 3)
        ]
    );
}

#[test]
fn with_esearch_the_answer_is_a_count_and_a_compact_set() {
    let trace = concat!(
        "# SYNTHETIC, shaped on RFC 4731 §3.1: RETURN (COUNT ALL) answered by ESEARCH, whose\n",
        "# correlator names the command. imap-proto has no grammar for it; the session keeps it\n",
        "# as text. A thousand matches are one short line.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 ESEARCH SPECIAL-USE\n",
        "S: a002 OK done\n",
        "C: a003 EXAMINE \"[Gmail]/All Mail\"\n",
        "S: * OK [UIDVALIDITY 11] UIDs valid\n",
        "S: a003 OK [READ-ONLY] done\n",
        "C: a004 UID SEARCH RETURN (COUNT ALL) OR SUBJECT \"lunch\" BODY \"lunch\"\n",
        "SPLIT\n",
        "S: * ESEARCH (TAG \"a004\") UID COUNT 1003 ALL 1:1000,2001:2003\n",
        "S: a004 OK Search completed\n",
        "DONE\n"
    );
    let mut backend = backend();
    let mut walk = backend.searching(plan(
        &["[Gmail]/All Mail"],
        vec![SearchKey::Or(
            Box::new(SearchKey::Subject("lunch".to_owned())),
            Box::new(SearchKey::Body("lunch".to_owned())),
        )],
    ));
    let found = replay(&mut walk, trace).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].count, 1003);
    assert_eq!(found[0].uids.len(), 1003);
    assert_eq!(found[0].uids.last(), Some(&2003));
    assert_eq!(found[0].uidvalidity, 11);
}

#[test]
fn a_string_that_is_not_ascii_waits_for_the_servers_go_ahead() {
    let trace = concat!(
        "# SYNTHETIC, shaped on RFC 3501 §7.5 and §6.4.4: no LITERAL+, so the literal is sent\n",
        "# only after the server's continuation, with CHARSET UTF-8 before the keys.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1\n",
        "S: a002 OK done\n",
        "C: a003 EXAMINE \"INBOX\"\n",
        "S: * OK [UIDVALIDITY 9] ok\n",
        "S: a003 OK [READ-ONLY] done\n",
        "C64: YTAwNCBVSUQgU0VBUkNIIENIQVJTRVQgVVRGLTggU1VCSkVDVCB7Nn0NCg==\n",
        "S: + go ahead\n",
        "C64: 5Y2I6aSQIFVOU0VFTg0K\n",
        "S: * SEARCH 8\n",
        "S: a004 OK done\n",
        "DONE\n"
    );
    let mut backend = backend();
    let mut walk = backend.searching(plan(
        &["INBOX"],
        vec![SearchKey::Subject("午餐".to_owned()), SearchKey::Unseen],
    ));
    assert_eq!(
        replay(&mut walk, trace).unwrap(),
        vec![found("INBOX", 9, &[8], 1)]
    );
}

#[test]
fn a_server_that_cannot_search_in_utf8_says_so_before_the_literal() {
    let trace = concat!(
        "# SYNTHETIC. RFC 3501 §6.4.4: an unsupported charset is a tagged NO with BADCHARSET,\n",
        "# which may come instead of the continuation. Nothing more is sent.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1\n",
        "S: a002 OK done\n",
        "C: a003 EXAMINE \"INBOX\"\n",
        "S: a003 OK [READ-ONLY] done\n",
        "C64: YTAwNCBVSUQgU0VBUkNIIENIQVJTRVQgVVRGLTggU1VCSkVDVCB7Nn0NCg==\n",
        "S: a004 NO [BADCHARSET (US-ASCII)] Unsupported charset\n",
        "FAIL Refused\n"
    );
    let mut backend = backend();
    let mut walk = backend.searching(plan(
        &["INBOX"],
        vec![SearchKey::Subject("午餐".to_owned()), SearchKey::Unseen],
    ));
    let e = replay(&mut walk, trace).unwrap_err();
    assert!(
        matches!(&e, ProtoError::Refused { text, .. } if text.contains("BADCHARSET")),
        "{e:?}"
    );
}

#[test]
fn nothing_to_search_opens_no_session() {
    let mut backend = backend();
    let mut walk = backend.searching(plan(&[], vec![SearchKey::All]));
    assert_eq!(replay(&mut walk, "DONE\n").unwrap(), Vec::<Found>::new());
}
