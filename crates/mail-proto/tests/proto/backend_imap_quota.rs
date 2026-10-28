//! Reading an IMAP account's storage, as bytes on the wire: `CAPABILITY`, then
//! `GETQUOTAROOT INBOX` (RFC 9208 §4.3.2) where the server offers `QUOTA`, and nothing where it
//! does not.

use crate::common;

use common::{password_backend, replay};
use mail_proto::{ProtoError, Storage};

#[test]
fn the_inbox_s_root_says_how_much_is_used_in_kibibytes() {
    let trace = concat!(
        "# SYNTHETIC, shaped on RFC 9208 §5.1 and §5.2: one root, counting storage and messages.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 QUOTA QUOTA=RES-STORAGE\n",
        "S: a002 OK done\n",
        "C: a003 GETQUOTAROOT \"INBOX\"\n",
        "S: * QUOTAROOT INBOX \"\"\n",
        "SPLIT\n",
        "S: * QUOTA \"\" (STORAGE 512 2048 MESSAGE 40 1000)\n",
        "S: a003 OK done\n",
        "DONE\n"
    );
    let mut backend = password_backend(common::caps());
    let mut walk = backend.quota();
    assert_eq!(
        replay(&mut walk, trace).unwrap(),
        Some(Storage {
            used: 512 * 1024,
            limit: 2048 * 1024,
        })
    );
}

#[test]
fn of_two_roots_the_one_with_less_left_is_said() {
    let trace = concat!(
        "# SYNTHETIC: the user's own root and the domain's, the domain nearly full.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 QUOTA\n",
        "S: a002 OK done\n",
        "C: a003 GETQUOTAROOT \"INBOX\"\n",
        "S: * QUOTAROOT INBOX \"user.ada\" \"example.test\"\n",
        "S: * QUOTA \"user.ada\" (STORAGE 10 1000)\n",
        "S: * QUOTA \"example.test\" (STORAGE 9990 10000)\n",
        "S: a003 OK done\n",
        "DONE\n"
    );
    let mut backend = password_backend(common::caps());
    let mut walk = backend.quota();
    assert_eq!(
        replay(&mut walk, trace).unwrap(),
        Some(Storage {
            used: 9990 * 1024,
            limit: 10000 * 1024,
        })
    );
}

#[test]
fn a_root_counting_only_messages_is_no_storage() {
    let trace = concat!(
        "# SYNTHETIC: a limit on how many messages, none on their size.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 QUOTA\n",
        "S: a002 OK done\n",
        "C: a003 GETQUOTAROOT \"INBOX\"\n",
        "S: * QUOTAROOT INBOX \"\"\n",
        "S: * QUOTA \"\" (MESSAGE 40 1000)\n",
        "S: a003 OK done\n",
        "DONE\n"
    );
    let mut backend = password_backend(common::caps());
    let mut walk = backend.quota();
    assert_eq!(replay(&mut walk, trace).unwrap(), None);
}

#[test]
fn without_quota_nothing_is_asked_and_there_is_nothing_to_say() {
    let trace = concat!(
        "# SYNTHETIC. No QUOTA after sign-in: GETQUOTAROOT would be BAD, so it is not sent, and\n",
        "# the trace ending here is the proof.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 OK Logged in\n",
        "C: a002 CAPABILITY\n",
        "S: * CAPABILITY IMAP4rev1 MOVE\n",
        "S: a002 OK done\n",
        "DONE\n"
    );
    let mut backend = password_backend(common::caps());
    let mut walk = backend.quota();
    assert_eq!(replay(&mut walk, trace).unwrap(), None);
}

#[test]
fn a_refused_sign_in_is_a_failure_not_an_account_without_quota() {
    let trace = concat!(
        "# SYNTHETIC: the password is refused before anything is asked.\n",
        "S: * OK ready\n",
        "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
        "S: a001 NO [AUTHENTICATIONFAILED] Invalid credentials\n",
        "FAIL AuthRejected\n"
    );
    let mut backend = password_backend(common::caps());
    let mut walk = backend.quota();
    let err = replay(&mut walk, trace).unwrap_err();
    assert!(!matches!(err, ProtoError::Unsupported(_)), "{err:?}");
}
