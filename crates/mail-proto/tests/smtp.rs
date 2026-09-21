//! SMTP submission, driven only by the shared trace harness.

mod common;

use common::replay;
use mail_domain::{Credential, SaslMech, Tls};
use mail_proto::{Advertised, EhloExtensions, ProtoError, Refusal, SmtpSession, Submission};

const USER: &str = "ada@example.com";
const PASSWORD: &str = "s3cr3t-password";
const TOKEN: &str = "ya29.s3cr3t-token";
const REFRESH: &str = "r3fresh-token-secret";
const PLAIN_B64: &str = "AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=";
const PASS_B64: &str = "czNjcjN0LXBhc3N3b3Jk";
const XOAUTH_B64: &str = "dXNlcj1hZGFAZXhhbXBsZS5jb20BYXV0aD1CZWFyZXIgeWEyOS5zM2NyM3QtdG9rZW4BAQ==";

const SHORT: &str = "Subject: hi\r\n\r\nhi\r\n";
const DOTTED: &str = "\
From: ada@example.com\r\n\
To: bob@example.com\r\n\
Subject: hello\r\n\
\r\n\
Hi.\r\n\
.this starts with a dot\r\n\
Bye\r\n";

fn expires() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp in range")
}

fn build(
    tls: Tls,
    port: u16,
    sasl: Vec<SaslMech>,
    credential: Credential,
    recipients: &[&str],
    message: &str,
) -> SmtpSession {
    SmtpSession::new(Submission {
        ehlo: "client.example".into(),
        host: "smtp.example".into(),
        port,
        tls,
        username: USER.into(),
        credential,
        sasl,
        mail_from: USER.into(),
        recipients: recipients.iter().map(|addr| (*addr).to_owned()).collect(),
        message: message.as_bytes().to_vec(),
    })
}

fn password() -> Credential {
    Credential::Password(PASSWORD.into())
}

fn oauth() -> Credential {
    Credential::OAuth {
        access: TOKEN.into(),
        refresh: REFRESH.into(),
        expires_at: expires(),
    }
}

fn plain(message: &str, recipients: &[&str]) -> SmtpSession {
    build(
        Tls::Implicit,
        465,
        vec![SaslMech::Plain],
        password(),
        recipients,
        message,
    )
}

fn assert_secrets_absent(rendered: &str) {
    for secret in [PASSWORD, TOKEN, REFRESH, PLAIN_B64, PASS_B64, XOAUTH_B64] {
        assert!(
            !rendered.contains(secret),
            "a credential appeared in a Debug or error string"
        );
    }
}

#[test]
fn submit_plain_stuffs_a_leading_dot_and_records_extensions() {
    let mut session = build(
        Tls::Implicit,
        465,
        vec![SaslMech::XOauth2, SaslMech::Plain, SaslMech::Login],
        password(),
        &["bob@example.com", "cara@example.com"],
        DOTTED,
    );
    let reply = replay(&mut session, include_str!("traces/smtp/submit.trace")).unwrap();
    assert_eq!(reply.mechanism, SaslMech::Plain);
    assert_eq!(
        reply.extensions.auth,
        vec![SaslMech::Plain, SaslMech::Login, SaslMech::XOauth2]
    );
    assert_eq!(
        reply.extensions.size,
        mail_proto::SizeLimit::Limited(35_882_577)
    );
    assert_eq!(reply.extensions.eight_bit_mime, Advertised::Offered);
    assert_eq!(reply.extensions.starttls, Advertised::Absent);
    assert_eq!(reply.accepted.code, 250);
    assert!(reply.accepted.text.contains("queued"));
    assert_eq!(reply.closing.as_ref().map(|c| c.code), Some(221));
    assert_secrets_absent(&format!("{session:?} {reply:?}"));
}

#[test]
fn auth_login_upgrades_with_starttls_and_answers_both_challenges() {
    let mut session = build(
        Tls::StartTlsRequired,
        587,
        vec![SaslMech::Plain, SaslMech::Login],
        password(),
        &["bob@example.com"],
        SHORT,
    );
    let reply = replay(&mut session, include_str!("traces/smtp/auth_login.trace")).unwrap();
    assert_eq!(reply.mechanism, SaslMech::Login);
    assert_eq!(reply.extensions.auth, vec![SaslMech::Login]);
    assert_eq!(reply.extensions.starttls, Advertised::Offered);
    assert_eq!(session.mechanism(), Some(SaslMech::Login));
}

#[test]
fn auth_xoauth2_skips_plain_when_the_credential_is_a_token() {
    let mut session = build(
        Tls::Implicit,
        465,
        vec![SaslMech::Plain, SaslMech::XOauth2],
        oauth(),
        &["bob@example.com"],
        SHORT,
    );
    let reply = replay(&mut session, include_str!("traces/smtp/auth_xoauth2.trace")).unwrap();
    assert_eq!(reply.mechanism, SaslMech::XOauth2);
    assert_eq!(
        reply.extensions.auth,
        vec![SaslMech::Plain, SaslMech::XOauth2]
    );
    let rendered = format!("{session:?} {reply:?}");
    assert!(rendered.contains("redacted"));
    assert_secrets_absent(&rendered);
}

#[test]
fn ehlo_split_byte_by_byte_still_submits() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("traces/smtp/ehlo_split.trace")).unwrap();
    assert_eq!(reply.mechanism, SaslMech::Plain);
    assert_eq!(reply.extensions.eight_bit_mime, Advertised::Offered);
    assert_eq!(
        reply.extensions,
        EhloExtensions {
            auth: vec![SaslMech::Plain, SaslMech::Login, SaslMech::XOauth2],
            starttls: Advertised::Absent,
            size: mail_proto::SizeLimit::Limited(35_882_577),
            eight_bit_mime: Advertised::Offered,
        }
    );
}

#[test]
fn auth_rejection_is_not_a_refusal_and_keeps_the_offered_mechanisms() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(
        &mut session,
        include_str!("traces/smtp/auth_rejected.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::AuthRejected(_)));
    assert_eq!(
        session.extensions().auth,
        vec![SaslMech::Plain, SaslMech::Login]
    );
    let rendered = format!("{err} {err:?} {session:?}");
    assert!(rendered.contains("redacted"));
    assert_secrets_absent(&rendered);
}

#[test]
fn refusals_distinguish_transient_from_permanent() {
    let cases = [
        (
            "rcpt 450",
            include_str!("traces/smtp/reject_transient.trace"),
            Refusal::Transient,
            "450",
        ),
        (
            "rcpt 550",
            include_str!("traces/smtp/reject_permanent.trace"),
            Refusal::Permanent,
            "550",
        ),
        (
            "auth 454",
            include_str!("traces/smtp/auth_temporary.trace"),
            Refusal::Transient,
            "454",
        ),
        (
            "greeting 554",
            include_str!("traces/smtp/reject_greeting.trace"),
            Refusal::Permanent,
            "554",
        ),
        (
            "size",
            include_str!("traces/smtp/reject_size.trace"),
            Refusal::Permanent,
            "message",
        ),
    ];
    for (name, trace, want_kind, expected) in cases {
        let mut session = plain(SHORT, &["bob@example.com"]);
        let err = replay(&mut session, trace).unwrap_err();
        let ProtoError::Refused { kind, text } = &err else {
            panic!("{name} was not a refusal");
        };
        // The class the outbox branches on: Transient backs off, Permanent undoes and gives up.
        assert_eq!(*kind, want_kind, "{name}");
        assert!(text.contains(expected), "{name}: {text}");
        assert_secrets_absent(&format!("{err} {err:?} {session:?}"));
    }
}

#[test]
fn an_unusable_mechanism_exposes_what_the_server_offered() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(
        &mut session,
        include_str!("traces/smtp/auth_unsupported.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
    assert_eq!(session.extensions().auth, vec![SaslMech::Login]);
    assert_eq!(session.mechanism(), None);
}

#[test]
fn eight_bit_messages_require_the_extension_and_declare_it() {
    let mut session = plain("café\r\n", &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("traces/smtp/eight_bit.trace")).unwrap();
    assert_eq!(reply.mechanism, SaslMech::Plain);
    assert_eq!(reply.extensions.eight_bit_mime, Advertised::Offered);

    let mut session = plain("café\r\n", &["bob@example.com"]);
    let err = replay(&mut session, include_str!("traces/smtp/need_8bit.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
}

#[test]
fn starttls_required_does_not_authenticate_in_cleartext() {
    let mut session = build(
        Tls::StartTlsRequired,
        587,
        vec![SaslMech::Login],
        password(),
        &["bob@example.com"],
        SHORT,
    );
    let err = replay(
        &mut session,
        include_str!("traces/smtp/starttls_missing.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
    assert_eq!(session.extensions().starttls, Advertised::Absent);
}

#[test]
fn a_malformed_greeting_and_an_early_close_fail_as_themselves() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("traces/smtp/malformed.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::Malformed(_)));

    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("traces/smtp/eof.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::UnexpectedEof));
}

#[test]
fn closing_after_acceptance_does_not_unsend_the_message() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("traces/smtp/quit_eof.trace")).unwrap();
    assert_eq!(reply.accepted.code, 250);
    assert!(reply.closing.is_none());

    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("traces/smtp/quit_rude.trace")).unwrap();
    assert_eq!(reply.accepted.code, 250);
    let closing = reply.closing.expect("QUIT was answered");
    assert_eq!(closing.code, 421);
}

#[test]
fn debug_does_not_contain_a_password_or_a_token() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(
        &mut session,
        include_str!("traces/smtp/auth_rejected.trace"),
    )
    .unwrap_err();
    let rendered = format!("{session:?}");
    assert!(rendered.contains("redacted"));
    assert!(rendered.contains(USER));
    assert_secrets_absent(&rendered);
    assert_secrets_absent(&format!("{err} {err:?}"));

    let token_session = build(
        Tls::Implicit,
        465,
        vec![SaslMech::XOauth2],
        oauth(),
        &["bob@example.com"],
        SHORT,
    );
    let rendered = format!("{token_session:?}");
    assert!(rendered.contains("redacted"));
    assert_secrets_absent(&rendered);
}

/// 421 must back off rather than be treated as a refusal.
///
/// A refusal is fatal, and the store answers fatal by applying the undo patch and dropping the
/// outbox entry — so misclassifying a rate limit discards the user's outgoing message. Gmail
/// rate-limits by daily volume and by simultaneous connections, so this is ordinary behaviour
/// rather than an edge case.
#[test]
fn a_rate_limit_backs_off_instead_of_discarding_the_message() {
    use mail_domain::{Retry, Retryable};
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("traces/smtp/throttled.trace")).unwrap_err();
    assert!(
        matches!(&err, ProtoError::Throttled { .. }),
        "421 must be Throttled, got {err:?}"
    );
    assert!(
        matches!(err.retry(), Retry::After(_)),
        "a throttle must schedule a retry, not give up"
    );
    assert_secrets_absent(&format!("{err} {err:?} {session:?}"));
}
