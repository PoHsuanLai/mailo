//! `ImapSession` against transcripts, several derived from a real Gmail capture.

mod common;

use common::replay;
use mail_domain::{Credential, SaslMech};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, ProtoError, Refusal};

fn oauth() -> ImapAuth {
    ImapAuth {
        username: "ada@example.test".to_owned(),
        credential: Credential::OAuth {
            access: "ya29.token".to_owned(),
            refresh: "1//refresh".to_owned(),
            expires_at: chrono::DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
        },
        sasl: vec![SaslMech::XOauth2],
    }
}

fn session(commands: Vec<ImapCommand>) -> ImapSession {
    ImapSession::new(oauth(), commands).expect("credentials are clean")
}

/// The greeting is untagged and must be absorbed before any command goes out.
#[test]
fn the_real_gmail_greeting_and_capability_parse() {
    let out = replay(
        &mut session(vec![ImapCommand::Capability]),
        include_str!("traces/imap/greeting_capability.trace"),
    )
    .unwrap();
    assert!(
        !out.capabilities.is_empty(),
        "the capability list should have been recorded"
    );
    let caps = format!("{:?}", out.capabilities);
    assert!(caps.contains("Idle") || caps.contains("IDLE"), "{caps}");
}

/// F14 as a test: the same server answers CAPABILITY differently once authenticated, and the
/// later answer must win. Believing the first is how a CONDSTORE server looks like it has none.
#[test]
fn a_later_capability_list_replaces_an_earlier_one() {
    let out = replay(
        &mut session(vec![
            ImapCommand::Capability,
            ImapCommand::AuthenticateXoauth2,
            ImapCommand::Capability,
        ]),
        include_str!("traces/imap/capability_grows_after_auth.trace"),
    )
    .unwrap();
    let caps = format!("{:?}", out.capabilities);
    assert!(
        caps.to_uppercase().contains("CONDSTORE"),
        "the post-auth list must win: {caps}"
    );
}

/// The reason imap-codec was rejected: Gmail's extension attributes must survive.
#[test]
fn gmail_fetch_attributes_reach_the_caller() {
    let out = replay(
        &mut session(vec![ImapCommand::UidFetch {
            set: "30048:30052".to_owned(),
            items: "(UID FLAGS X-GM-MSGID X-GM-THRID X-GM-LABELS)".to_owned(),
        }]),
        include_str!("traces/imap/gmail_fetch.trace"),
    )
    .unwrap();
    let all = out
        .untagged
        .iter()
        .map(|u| u.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needed in ["1876965017123110734", "travel", "32460"] {
        assert!(all.contains(needed), "lost {needed} in:\n{all}");
    }
}

/// RFC 3501 7.4.1 permits an untagged EXPUNGE during a UID command. It must be accepted where
/// it lands, and no sequence number may reach the caller as an identity.
#[test]
fn an_unsolicited_expunge_mid_command_is_absorbed() {
    let out = replay(
        &mut session(vec![ImapCommand::UidFetch {
            set: "1:*".to_owned(),
            items: "(UID FLAGS)".to_owned(),
        }]),
        include_str!("traces/imap/unsolicited_expunge.trace"),
    )
    .unwrap();
    let texts: Vec<&str> = out.untagged.iter().map(|u| u.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("EXPUNGE")),
        "the expunge must be recorded, not discarded: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("UID 103")),
        "and the command must continue afterwards: {texts:?}"
    );
}

/// The cancellation path the Progress design exists for.
#[test]
fn idle_is_interrupted_in_protocol_rather_than_dropped() {
    replay(
        &mut session(vec![ImapCommand::Idle]),
        include_str!("traces/imap/idle_interrupted.trace"),
    )
    .unwrap();
}

/// Mozilla 344205: a server advertising IDLE and answering NO wedged Thunderbird for years,
/// because it assumed the continuation and sent DONE anyway.
#[test]
fn idle_refused_does_not_send_done() {
    let err = replay(
        &mut session(vec![ImapCommand::Idle]),
        include_str!("traces/imap/idle_refused.trace"),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ProtoError::Refused {
                kind: Refusal::Permanent,
                ..
            }
        ),
        "{err:?}"
    );
}

/// A rate limit must back off, not discard the user's work.
#[test]
fn a_limit_response_is_throttled_not_refused() {
    use mail_domain::{Retry, Retryable};
    let err = replay(
        &mut session(vec![ImapCommand::UidFetch {
            set: "1:*".to_owned(),
            items: "(UID)".to_owned(),
        }]),
        include_str!("traces/imap/rate_limited.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Throttled { .. }), "{err:?}");
    assert!(matches!(err.retry(), Retry::After(_)));
}

/// Real sockets deliver a FETCH across many reads.
#[test]
fn a_fetch_split_one_byte_at_a_time_is_reassembled() {
    let out = replay(
        &mut session(vec![ImapCommand::UidFetch {
            set: "1".to_owned(),
            items: "(UID FLAGS)".to_owned(),
        }]),
        include_str!("traces/imap/fetch_split.trace"),
    )
    .unwrap();
    assert!(out.untagged.iter().any(|u| u.text.contains("UID 101")));
}

// --- things that must be refused ------------------------------------------------------

/// A credential containing CR or LF would split a command. That is IMAP injection.
#[test]
fn a_credential_with_a_newline_is_refused_at_construction() {
    let auth = ImapAuth {
        username: "ada@example.test".to_owned(),
        credential: Credential::Password("pass\r\na001 DELETE INBOX".to_owned()),
        sasl: vec![SaslMech::Plain],
    };
    assert!(ImapSession::new(auth, vec![ImapCommand::Login]).is_err());
}

/// A UID set is built from stored values; an unchecked one is a way to append command text.
#[test]
fn a_uid_set_that_is_not_a_uid_set_is_refused() {
    for bad in ["", "1:* (UID)\r\na002 DELETE INBOX", "abc", "1 2"] {
        let err = replay(
            &mut session(vec![ImapCommand::UidFetch {
                set: bad.to_owned(),
                items: "(UID)".to_owned(),
            }]),
            concat!("S: * OK Gimap ready\n", "FAIL Malformed\n"),
        );
        assert!(
            matches!(err, common::Ended::Failed(_)),
            "{bad:?} should have been refused"
        );
    }
}

/// XOAUTH2 with a password credential is a configuration error, not something to attempt.
#[test]
fn xoauth2_without_an_oauth_credential_is_unsupported() {
    let auth = ImapAuth {
        username: "ada@example.test".to_owned(),
        credential: Credential::Password("hunter2".to_owned()),
        sasl: vec![SaslMech::XOauth2],
    };
    let mut s = ImapSession::new(auth, vec![ImapCommand::AuthenticateXoauth2]).unwrap();
    let outcome = replay(
        &mut s,
        concat!("S: * OK Gimap ready\n", "FAIL Unsupported\n"),
    );
    assert!(matches!(outcome, common::Ended::Failed(_)));
}

/// Nothing may print a token.
#[test]
fn debug_does_not_leak_the_bearer_token() {
    let s = session(vec![ImapCommand::Capability]);
    let rendered = format!("{s:?} {:?}", oauth());
    assert!(!rendered.contains("ya29.token"), "{rendered}");
    assert!(!rendered.contains("1//refresh"), "{rendered}");
    assert!(rendered.contains("redacted"));
}
