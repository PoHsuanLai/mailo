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

/// Mail that landed between the last sync pass and this watch is in SELECT's `UIDNEXT`, and
/// the walk ends there: `IDLE` would only have announced what arrived after it began.
#[test]
fn a_watch_that_selects_past_what_was_synced_does_not_idle() {
    let watch = |uidnext| {
        session(vec![
            ImapCommand::Select {
                mailbox: "INBOX".to_owned(),
                read_only: true,
                qresync: None,
            },
            ImapCommand::IdleAfter { uidnext },
        ])
    };
    replay(
        &mut watch(5),
        include_str!("traces/imap/idle_after_new_mail.trace"),
    )
    .unwrap();
    replay(
        &mut watch(5),
        include_str!("traces/imap/idle_after_nothing_new.trace"),
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

/// What a response that does not parse tells the person reading the error.
mod unreadable_responses {
    use super::*;
    use mail_proto::machine::{IoReady, Machine, Progress};

    fn started(commands: Vec<ImapCommand>) -> ImapSession {
        let mut s = session(commands);
        let _ = s.start();
        let _ = s.feed(IoReady::Bytes(b"* OK ready\r\n".to_vec()));
        s
    }

    #[test]
    fn a_malformed_response_is_quoted_as_text_not_as_numbers() {
        // It used to print `nom`'s `Error { input: [42, 32, 49, ...] }` — five hundred decimal
        // integers where the answer is one line of IMAP. That is the only diagnostic the field
        // ever sees, and nobody can read it.
        let mut session = started(vec![ImapCommand::Capability]);
        let progress = session.feed(IoReady::Bytes(
            b"* 1 FETCH (BODYSTRUCTURE ((\"text\" \"plain\") (\"text\" \"html\") \"alternative\"))\r\n".to_vec(),
        ));

        let text = match progress {
            Progress::Failed(e) => format!("{e}"),
            other => panic!("that response should not parse: {other:?}"),
        };
        assert!(text.contains("BODYSTRUCTURE"), "{text}");
        assert!(text.contains("alternative"), "{text}");
        assert!(
            !text.contains("[42,") && !text.contains("input:"),
            "still printing nom's byte array: {text}"
        );
        // Line breaks are escaped rather than printed, so one error stays one line.
        assert!(!text.contains('\n'), "{text}");
    }

    #[test]
    fn a_very_long_response_is_cut_rather_than_dumped() {
        let mut session = started(vec![ImapCommand::Capability]);
        let mut junk = b"* 1 FETCH (BODYSTRUCTURE ".to_vec();
        junk.extend(std::iter::repeat_n(b'x', 5000));
        junk.extend_from_slice(b"\r\n");
        let progress = session.feed(IoReady::Bytes(junk));

        let text = match progress {
            Progress::Failed(e) => format!("{e}"),
            other => panic!("{other:?}"),
        };
        assert!(text.len() < 600, "the error is {} characters", text.len());
        assert!(
            text.contains("bytes)"),
            "it says how much was left out: {text}"
        );
    }
}

/// How a `NO` to a login is classified, across servers that word it differently.
///
/// This decides whether the client keeps trying. `Retry::NeedsReauth` stops the poll loop and
/// asks the user to fix the credential; anything else is retried on a timer. A wrong password
/// retried every five minutes is 288 failed logins a day against the user's own mail server,
/// which is how an account gets locked out — so the classification has to hold for servers that
/// never learned Dovecot's vocabulary, not just for the two whose wording was to hand.
mod a_refused_login {
    use super::*;
    use mail_domain::{Retry, Retryable};

    fn refusing_with(text: &str) -> ProtoError {
        let auth = ImapAuth {
            username: "ada@example.test".to_owned(),
            credential: Credential::Password("wrong".to_owned()),
            sasl: vec![SaslMech::Plain],
        };
        let mut s = ImapSession::new(auth, vec![ImapCommand::Login]).unwrap();
        match replay(
            &mut s,
            &format!(
                "S: * OK ready\n\
                 C: a001 LOGIN \"ada@example.test\" \"wrong\"\n\
                 S: {text}\n\
                 FAIL\n"
            ),
        ) {
            common::Ended::Failed(e) => e,
            other => panic!("{text:?} should have failed the session: {other:?}"),
        }
    }

    #[test]
    fn every_servers_wording_means_the_credential_must_be_fixed() {
        // Left to right: Dovecot, Gmail, Exchange/Office 365, Courier and UW-imapd, Zimbra,
        // Cyrus. Only the first two say anything a substring search for "AUTHENTICATIONFAILED"
        // or "invalid credentials" would find.
        for text in [
            "a001 NO [AUTHENTICATIONFAILED] Authentication failed.",
            "a001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            "a001 NO LOGIN failed.",
            "a001 NO Login failed.",
            "a001 NO LOGIN failed",
            "a001 NO Login incorrect",
        ] {
            let err = refusing_with(text);
            assert!(
                matches!(err.retry(), Retry::NeedsReauth),
                "{text:?} was classified {err:?}, so the client would keep trying the same \
                 password on a timer"
            );
        }
    }

    /// The other direction, which is what keeps the rule honest: a refusal that is not about the
    /// credential must not be reported as one, or every ordinary rejection sends the user off to
    /// re-run `account add` for nothing.
    #[test]
    fn a_refusal_that_is_not_about_the_credential_is_not_one() {
        let auth = ImapAuth {
            username: "ada@example.test".to_owned(),
            credential: Credential::Password("right".to_owned()),
            sasl: vec![SaslMech::Plain],
        };
        let mut s = ImapSession::new(
            auth,
            vec![
                ImapCommand::Login,
                ImapCommand::Select {
                    mailbox: "Archive".to_owned(),
                    read_only: false,
                    qresync: None,
                },
            ],
        )
        .unwrap();
        let err = match replay(
            &mut s,
            concat!(
                "S: * OK ready\n",
                "C: a001 LOGIN \"ada@example.test\" \"right\"\n",
                "S: a001 OK LOGIN completed\n",
                "C: a002 SELECT \"Archive\"\n",
                "S: a002 NO Mailbox does not exist\n",
                "FAIL Refused\n",
            ),
        ) {
            common::Ended::Failed(e) => e,
            other => panic!("{other:?}"),
        };
        assert!(
            !matches!(err.retry(), Retry::NeedsReauth),
            "a missing mailbox was blamed on the password: {err:?}"
        );
    }
}
