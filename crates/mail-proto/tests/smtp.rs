//! SMTP submission, driven only by the shared trace harness.

mod common;

use common::replay;
use mail_domain::{Credential, SaslMech, Tls};
use mail_proto::smtp::{Notify, Receipt, Return};
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
        receipt: None,
        message: message.as_bytes().to_vec(),
    })
}

fn password() -> Credential {
    Credential::Password(PASSWORD.into())
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
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

fn envelope(mail_from: &str, recipients: &[&str], message: &str) -> SmtpSession {
    SmtpSession::new(Submission {
        ehlo: "client.example".into(),
        host: "smtp.example".into(),
        port: 465,
        tls: Tls::Implicit,
        username: USER.into(),
        credential: password(),
        sasl: vec![SaslMech::Plain],
        mail_from: mail_from.into(),
        recipients: recipients.iter().map(|addr| (*addr).to_owned()).collect(),
        receipt: None,
        message: message.as_bytes().to_vec(),
    })
}

fn with_receipt(
    mail_from: &str,
    recipients: &[&str],
    message: &str,
    receipt: Option<Receipt>,
) -> SmtpSession {
    SmtpSession::new(Submission {
        ehlo: "client.example".into(),
        host: "smtp.example".into(),
        port: 465,
        tls: Tls::Implicit,
        username: USER.into(),
        credential: password(),
        sasl: vec![SaslMech::Plain],
        mail_from: mail_from.into(),
        recipients: recipients.iter().map(|addr| (*addr).to_owned()).collect(),
        receipt,
        message: message.as_bytes().to_vec(),
    })
}

/// ASCII envelope, server lists `SMTPUTF8` in mixed case.
///
/// `MAIL FROM` must not gain the parameter: the addresses are ASCII, and the
/// command has to stay the one an older server already accepts.
const ASCII_SMTPUTF8: &str = "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-Smtputf8
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: Subject: hi
C:
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
";

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
            smtputf8: Advertised::Absent,
            dsn: Advertised::Absent,
            chunking: Advertised::Absent,
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

#[test]
fn ehlo_parses_smtputf8() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, ASCII_SMTPUTF8).unwrap();
    assert_eq!(reply.extensions.smtputf8, Advertised::Offered);
}

#[test]
fn an_ascii_submission_omits_smtputf8_even_when_the_server_offers_it() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, ASCII_SMTPUTF8).unwrap();
    // The trace's MAIL FROM line is the assertion that the parameter is absent.
    // This guards the other half: the omission is not because parsing missed it.
    assert_eq!(reply.extensions.smtputf8, Advertised::Offered);
}

#[test]
fn a_utf8_address_is_sent_raw_when_smtputf8_is_offered() {
    let mut session = envelope("用户@例子.广告", &["收件人@例子.广告"], "hi\r\n");
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-SMTPUTF8
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<用户@例子.广告> SMTPUTF8
S: 250 2.1.0 OK
C: RCPT TO:<收件人@例子.广告>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.smtputf8, Advertised::Offered);
}

#[test]
fn a_utf8_domain_is_rewritten_to_an_a_label_without_smtputf8() {
    let mut session = envelope("ada@bücher.example", &["bob@例子.广告"], "hi\r\n");
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@xn--bcher-kva.example>
S: 250 2.1.0 OK
C: RCPT TO:<bob@xn--fsqu00a.xn--4rr70v>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.smtputf8, Advertised::Absent);
}

#[test]
fn a_utf8_local_part_is_unsupported_without_smtputf8_and_mail_from_is_not_sent() {
    // The transcript stops at EHLO. A MAIL FROM, or even AUTH, would still be
    // a write the trace does not expect, so the refusal is before either.
    let mut session = envelope("jörg@example.com", &["bob@example.com"], "hi\r\n");
    let err = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250 AUTH PLAIN
FAIL Unsupported
",
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "server does not support SMTPUTF8, which jörg@example.com needs: its local part is not ASCII"
    );
}

#[test]
fn eight_bit_body_and_utf8_address_declare_both_parameters() {
    let mut session = envelope("用户@例子.广告", &["bob@例子.广告"], "café\r\n");
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-8BITMIME
S: 250-SMTPUTF8
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<用户@例子.广告> BODY=8BITMIME SMTPUTF8
S: 250 2.1.0 OK
C: RCPT TO:<bob@例子.广告>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: café
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.smtputf8, Advertised::Offered);
    assert_eq!(reply.extensions.eight_bit_mime, Advertised::Offered);
}

fn headers_receipt() -> Receipt {
    Receipt {
        notify: Notify::On {
            success: true,
            failure: true,
            delay: false,
        },
        ret: Return::Headers,
        envid: Some("q+1=x".into()),
    }
}

#[test]
fn an_absent_receipt_leaves_the_commands_unchanged_when_dsn_is_offered() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-DSN
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: Subject: hi
C:
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    // The trace's MAIL FROM and RCPT TO lines are the assertion that nothing
    // was added. This guards the other half: the omission is not a missed keyword.
    assert_eq!(reply.extensions.dsn, Advertised::Offered);
}

#[test]
fn a_receipt_adds_xtext_parameters_when_dsn_is_offered() {
    let mut session = with_receipt(USER, &["bob@example.com"], SHORT, Some(headers_receipt()));
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-DSN
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com> RET=HDRS ENVID=q+2B1+3Dx
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com> NOTIFY=SUCCESS,FAILURE ORCPT=rfc822;bob@example.com
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: Subject: hi
C:
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.dsn, Advertised::Offered);
    assert_eq!(reply.accepted.code, 250);
}

#[test]
fn a_receipt_is_omitted_without_failing_when_dsn_is_not_offered() {
    let mut session = with_receipt(USER, &["bob@example.com"], SHORT, Some(headers_receipt()));
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: Subject: hi
C:
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.dsn, Advertised::Absent);
    assert_eq!(reply.accepted.code, 250);
}

#[test]
fn a_non_ascii_recipient_gets_notify_and_no_orcpt() {
    let mut session = with_receipt(
        USER,
        &["收件人@例子.广告"],
        "hi\r\n",
        Some(Receipt {
            notify: Notify::On {
                success: false,
                failure: true,
                delay: false,
            },
            ret: Return::Full,
            envid: None,
        }),
    );
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-SMTPUTF8
S: 250-DSN
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com> SMTPUTF8 RET=FULL
S: 250 2.1.0 OK
C: RCPT TO:<收件人@例子.广告> NOTIFY=FAILURE
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.dsn, Advertised::Offered);
    assert_eq!(reply.extensions.smtputf8, Advertised::Offered);
}

/// `DOTTED` contains a line that begins with `.`. `DATA` would stuff it;
/// `BDAT` must write that line as it stands, because the count is the raw length.
#[test]
fn chunking_sends_one_bdat_and_does_not_dot_stuff() {
    let message = DOTTED;
    let trace = format!(
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-CHUNKING
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: BDAT {n} LAST
C64: {body}
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
        n = message.len(),
        body = b64(message.as_bytes()),
    );
    let mut session = plain(message, &["bob@example.com"]);
    let reply = replay(&mut session, &trace).unwrap();
    assert_eq!(reply.extensions.chunking, Advertised::Offered);
    assert_eq!(reply.accepted.code, 250);
}

#[test]
fn without_chunking_a_leading_dot_is_still_stuffed_after_data() {
    let mut session = plain(DOTTED, &["bob@example.com"]);
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: From: ada@example.com
C: To: bob@example.com
C: Subject: hello
C:
C: Hi.
C: ..this starts with a dot
C: Bye
C: .
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.chunking, Advertised::Absent);
    assert_eq!(reply.accepted.code, 250);
}

#[test]
fn a_rejected_bdat_is_a_permanent_refusal() {
    let mut session = plain("hi\r\n", &["bob@example.com"]);
    let err = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-CHUNKING
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: BDAT 4 LAST
C: hi
S: 550 5.0.0 no
FAIL Refused
",
    )
    .unwrap_err();
    let ProtoError::Refused { kind, text } = &err else {
        panic!("BDAT 550 was not a refusal: {err:?}");
    };
    assert_eq!(*kind, Refusal::Permanent);
    assert!(text.contains("550"), "{text}");
}

/// `.x` stuffs to five octets. `SIZE 4` refuses that on `DATA` and accepts the
/// four raw octets on `BDAT`.
#[test]
fn chunking_measures_size_on_the_raw_message() {
    let message = ".x\r\n";
    let mut session = plain(message, &["bob@example.com"]);
    let reply = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-SIZE 4
S: 250-CHUNKING
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: BDAT 4 LAST
C: .x
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
    )
    .unwrap();
    assert_eq!(reply.extensions.chunking, Advertised::Offered);
    assert_eq!(reply.accepted.code, 250);

    let mut session = plain(message, &["bob@example.com"]);
    let err = replay(
        &mut session,
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-SIZE 4
S: 250 AUTH PLAIN
FAIL Refused
",
    )
    .unwrap_err();
    let ProtoError::Refused { kind, text } = &err else {
        panic!("the stuffed body should have been refused: {err:?}");
    };
    assert_eq!(*kind, Refusal::Permanent);
    assert!(text.contains("5 octets"), "{text}");
    assert!(text.contains("SIZE 4"), "{text}");
}

#[test]
fn bdat_sends_a_message_that_does_not_end_in_crlf() {
    let message = "no-crlf";
    let trace = format!(
        "\
S: 220 smtp.example ESMTP ready
C: EHLO client.example
S: 250-smtp.example Hello
S: 250-CHUNKING
S: 250 AUTH PLAIN
C: AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBzM2NyM3QtcGFzc3dvcmQ=
S: 235 2.7.0 Authentication successful
C: MAIL FROM:<ada@example.com>
S: 250 2.1.0 OK
C: RCPT TO:<bob@example.com>
S: 250 2.1.5 OK
C: BDAT {n} LAST
C64: {body}
S: 250 2.0.0 OK queued
C: QUIT
S: 221 2.0.0 Bye
DONE
",
        n = message.len(),
        body = b64(message.as_bytes()),
    );
    let mut session = plain(message, &["bob@example.com"]);
    let reply = replay(&mut session, &trace).unwrap();
    assert_eq!(reply.accepted.code, 250);
}
