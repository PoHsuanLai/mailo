//! SMTP submission, driven only by the shared trace harness.

use crate::common;

use common::replay;
use mail_domain::{SaslMech, Tls};
use mail_proto::smtp::{Notify, Receipt, Return};
use mail_proto::{
    Advertised, Authentication, EhloExtensions, ProtoError, Refusal, SignIn, SmtpSession,
    Submission,
};
use porter_core::Credential;
use porter_core::SecretText;
use porter_ids::UnixSeconds;

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
        auth: Authentication::SignIn(SignIn {
            username: USER.into(),
            credential,
            sasl,
        }),
        mail_from: USER.into(),
        recipients: recipients.iter().map(|addr| (*addr).to_owned()).collect(),
        receipt: None,
        message: message.as_bytes().to_vec(),
    })
}

fn password() -> Credential {
    Credential::Password(SecretText::new(PASSWORD))
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn oauth() -> Credential {
    Credential::OAuth {
        access: SecretText::new(TOKEN),
        refresh: SecretText::new(REFRESH),
        expires_at: UnixSeconds((expires()).timestamp()),
        granted: Default::default(),
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

fn envelope(
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
        auth: Authentication::SignIn(SignIn {
            username: USER.into(),
            credential: password(),
            sasl: vec![SaslMech::Plain],
        }),
        mail_from: mail_from.into(),
        recipients: recipients.iter().map(|addr| (*addr).to_owned()).collect(),
        receipt,
        message: message.as_bytes().to_vec(),
    })
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
    let reply = replay(&mut session, include_str!("../traces/smtp/submit.trace")).unwrap();
    assert_eq!(reply.mechanism, Some(SaslMech::Plain));
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

/// On a porter relay's connection the session never authenticates, and does not ask for
/// `STARTTLS` the plan says the real server wants: the relay owns both (`Authentication::Relayed`).
#[test]
fn a_relayed_session_goes_from_ehlo_to_mail_from_without_auth_or_starttls() {
    let mut session = SmtpSession::new(Submission {
        ehlo: "client.example".into(),
        host: "smtp.example".into(),
        port: 587,
        tls: Tls::StartTlsRequired,
        auth: Authentication::Relayed,
        mail_from: USER.into(),
        recipients: vec!["bob@example.com".to_owned()],
        receipt: None,
        message: SHORT.as_bytes().to_vec(),
    });
    let reply = replay(&mut session, include_str!("../traces/smtp/relayed.trace")).unwrap();
    assert_eq!(reply.mechanism, None);
    assert_eq!(reply.extensions.auth, vec![]);
    assert_eq!(reply.accepted.code, 250);
    assert_secrets_absent(&format!("{session:?} {reply:?}"));
}

/// The request a draft's "Request Delivery Receipt" makes (`mail_runtime`'s engine): delivery and
/// failure, headers only, no envelope id.
fn delivery_receipt() -> Receipt {
    Receipt {
        notify: Notify::On {
            success: true,
            failure: true,
            delay: false,
        },
        ret: Return::Headers,
        envid: None,
    }
}

#[test]
fn a_delivery_receipt_asks_notify_where_dsn_is_offered_and_is_left_off_where_not() {
    let recipients = ["bob@example.com", "cara@example.com"];
    let offered = replay(
        &mut envelope(USER, &recipients, SHORT, Some(delivery_receipt())),
        include_str!("../traces/smtp/dsn.trace"),
    )
    .unwrap();
    assert_eq!(offered.extensions.dsn, Advertised::Offered);
    // Skipped silently: no error, and the message is accepted all the same.
    let absent = replay(
        &mut envelope(USER, &recipients, SHORT, Some(delivery_receipt())),
        include_str!("../traces/smtp/dsn_absent.trace"),
    )
    .unwrap();
    assert_eq!(absent.extensions.dsn, Advertised::Absent);
    assert_eq!(absent.accepted.code, 250);
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
    let reply = replay(
        &mut session,
        include_str!("../traces/smtp/auth_login.trace"),
    )
    .unwrap();
    assert_eq!(reply.mechanism, Some(SaslMech::Login));
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
    let reply = replay(
        &mut session,
        include_str!("../traces/smtp/auth_xoauth2.trace"),
    )
    .unwrap();
    assert_eq!(reply.mechanism, Some(SaslMech::XOauth2));
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
    let reply = replay(
        &mut session,
        include_str!("../traces/smtp/ehlo_split.trace"),
    )
    .unwrap();
    assert_eq!(reply.mechanism, Some(SaslMech::Plain));
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
        include_str!("../traces/smtp/auth_rejected.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::AuthRejected(_)));
    assert_eq!(
        session.extensions().auth,
        vec![SaslMech::Plain, SaslMech::Login]
    );
    assert!(format!("{session:?}").contains(USER));
    let rendered = format!("{err} {err:?} {session:?}");
    assert!(rendered.contains("redacted"));
    assert_secrets_absent(&rendered);
}

#[test]
fn refusals_distinguish_transient_from_permanent() {
    let cases = [
        (
            "rcpt 450",
            include_str!("../traces/smtp/reject_transient.trace"),
            Refusal::Transient,
            "450",
        ),
        (
            "rcpt 550",
            include_str!("../traces/smtp/reject_permanent.trace"),
            Refusal::Permanent,
            "550",
        ),
        (
            "auth 454",
            include_str!("../traces/smtp/auth_temporary.trace"),
            Refusal::Transient,
            "454",
        ),
        (
            "greeting 554",
            include_str!("../traces/smtp/reject_greeting.trace"),
            Refusal::Permanent,
            "554",
        ),
        (
            "size",
            include_str!("../traces/smtp/reject_size.trace"),
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
        include_str!("../traces/smtp/auth_unsupported.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
    assert_eq!(session.extensions().auth, vec![SaslMech::Login]);
    assert_eq!(session.mechanism(), None);
}

#[test]
fn eight_bit_messages_require_the_extension_and_declare_it() {
    let mut session = plain("café\r\n", &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("../traces/smtp/eight_bit.trace")).unwrap();
    assert_eq!(reply.mechanism, Some(SaslMech::Plain));
    assert_eq!(reply.extensions.eight_bit_mime, Advertised::Offered);

    let mut session = plain("café\r\n", &["bob@example.com"]);
    let err = replay(&mut session, include_str!("../traces/smtp/need_8bit.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
}

#[test]
fn a_line_over_998_octets_is_refused_permanently_before_anything_is_sent() {
    // RFC 5321 §4.5.3.1.6: 1000 octets with the CRLF. Past it a server truncates or rejects, and
    // retrying cannot help, so the refusal is permanent and arrives before AUTH.
    let long = format!("Subject: hi\r\n\r\n{}\r\n", "x".repeat(999));
    let mut session = plain(&long, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("../traces/smtp/long_line.trace")).unwrap_err();
    let ProtoError::Refused { kind, text } = &err else {
        panic!("expected a refusal: {err:?}");
    };
    assert_eq!(*kind, Refusal::Permanent);
    assert!(text.contains("999"), "{text}");
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
        include_str!("../traces/smtp/starttls_missing.trace"),
    )
    .unwrap_err();
    assert!(matches!(err, ProtoError::Unsupported(_)));
    assert_eq!(session.extensions().starttls, Advertised::Absent);
}

#[test]
fn a_malformed_greeting_and_an_early_close_fail_as_themselves() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("../traces/smtp/malformed.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::Malformed(_)));

    let mut session = plain(SHORT, &["bob@example.com"]);
    let err = replay(&mut session, include_str!("../traces/smtp/eof.trace")).unwrap_err();
    assert!(matches!(err, ProtoError::UnexpectedEof));
}

#[test]
fn closing_after_acceptance_does_not_unsend_the_message() {
    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("../traces/smtp/quit_eof.trace")).unwrap();
    assert_eq!(reply.accepted.code, 250);
    assert!(reply.closing.is_none());

    let mut session = plain(SHORT, &["bob@example.com"]);
    let reply = replay(&mut session, include_str!("../traces/smtp/quit_rude.trace")).unwrap();
    assert_eq!(reply.accepted.code, 250);
    let closing = reply.closing.expect("QUIT was answered");
    assert_eq!(closing.code, 421);
}

/// A session holding a bearer token shows neither it nor its refresh token, even before it runs.
#[test]
fn debug_does_not_contain_a_token() {
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
    let err = replay(&mut session, include_str!("../traces/smtp/throttled.trace")).unwrap_err();
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

/// A submission the server takes: the greeting, `EHLO` answered with `extensions` (each a
/// `250-` line) and `AUTH PLAIN`, the sign-in, then `envelope` from `MAIL FROM` to the message's
/// end, queued, and `QUIT`.
fn taken(extensions: &[&str], envelope: &str) -> String {
    let mut trace = String::from(
        "S: 220 smtp.example ESMTP ready\nC: EHLO client.example\nS: 250-smtp.example Hello\n",
    );
    for extension in extensions {
        trace.push_str(&format!("S: 250-{extension}\n"));
    }
    trace.push_str(&format!(
        "S: 250 AUTH PLAIN\nC: AUTH PLAIN {PLAIN_B64}\nS: 235 2.7.0 Authentication successful\n"
    ));
    trace.push_str(envelope);
    trace.push_str("S: 250 2.0.0 OK queued\nC: QUIT\nS: 221 2.0.0 Bye\nDONE\n");
    trace
}

/// `MAIL FROM` to the message's end for one `BDAT`, which carries `message` as it stands.
fn bdat(message: &str) -> String {
    format!(
        "C: MAIL FROM:<ada@example.com>\n\
         S: 250 2.1.0 OK\n\
         C: RCPT TO:<bob@example.com>\n\
         S: 250 2.1.5 OK\n\
         C: BDAT {} LAST\n\
         C64: {}\n",
        message.len(),
        b64(message.as_bytes()),
    )
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

/// `(what it shows, the session, the extensions the server lists, the envelope it is sent)`.
///
/// The `MAIL FROM` and `RCPT TO` lines are the assertion: which parameters go, in which form, and
/// which are left off. The test checks the other half, that each extension was read as listed,
/// so an omission is never a missed keyword.
fn submissions() -> Vec<(&'static str, SmtpSession, &'static [&'static str], String)> {
    let dsn_only = |receipt| envelope(USER, &["bob@example.com"], SHORT, receipt);
    let short = "\
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
";
    vec![
        (
            "an ASCII submission omits SMTPUTF8 even when the server lists it, in mixed case",
            plain(SHORT, &["bob@example.com"]),
            &["Smtputf8"],
            short.to_owned(),
        ),
        (
            "a UTF-8 address is sent raw when SMTPUTF8 is offered",
            envelope("用户@例子.广告", &["收件人@例子.广告"], "hi\r\n", None),
            &["SMTPUTF8"],
            "\
C: MAIL FROM:<用户@例子.广告> SMTPUTF8
S: 250 2.1.0 OK
C: RCPT TO:<收件人@例子.广告>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
"
            .to_owned(),
        ),
        (
            "a UTF-8 domain is rewritten to an A-label without SMTPUTF8",
            envelope("ada@bücher.example", &["bob@例子.广告"], "hi\r\n", None),
            &[],
            "\
C: MAIL FROM:<ada@xn--bcher-kva.example>
S: 250 2.1.0 OK
C: RCPT TO:<bob@xn--fsqu00a.xn--4rr70v>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
"
            .to_owned(),
        ),
        (
            "an 8-bit body and a UTF-8 address declare both parameters",
            envelope("用户@例子.广告", &["bob@例子.广告"], "café\r\n", None),
            &["8BITMIME", "SMTPUTF8"],
            "\
C: MAIL FROM:<用户@例子.广告> BODY=8BITMIME SMTPUTF8
S: 250 2.1.0 OK
C: RCPT TO:<bob@例子.广告>
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: café
C: .
"
            .to_owned(),
        ),
        (
            "an absent receipt leaves the commands unchanged when DSN is offered",
            plain(SHORT, &["bob@example.com"]),
            &["DSN"],
            short.to_owned(),
        ),
        (
            "a receipt adds xtext parameters when DSN is offered",
            dsn_only(Some(headers_receipt())),
            &["DSN"],
            "\
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
"
            .to_owned(),
        ),
        (
            "a receipt is omitted without failing when DSN is not offered",
            dsn_only(Some(headers_receipt())),
            &[],
            short.to_owned(),
        ),
        (
            "a non-ASCII recipient gets NOTIFY and no ORCPT",
            envelope(
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
            ),
            &["SMTPUTF8", "DSN"],
            "\
C: MAIL FROM:<ada@example.com> SMTPUTF8 RET=FULL
S: 250 2.1.0 OK
C: RCPT TO:<收件人@例子.广告> NOTIFY=FAILURE
S: 250 2.1.5 OK
C: DATA
S: 354 go ahead
C: hi
C: .
"
            .to_owned(),
        ),
        // `DOTTED` has a line that begins with `.`. `DATA` stuffs it; `BDAT` writes it as it
        // stands, because the count is the raw length.
        (
            "with CHUNKING one BDAT goes, not dot-stuffed",
            plain(DOTTED, &["bob@example.com"]),
            &["CHUNKING"],
            bdat(DOTTED),
        ),
        (
            "without CHUNKING a leading dot is still stuffed after DATA",
            plain(DOTTED, &["bob@example.com"]),
            &[],
            "\
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
"
            .to_owned(),
        ),
        // `.x` stuffs to five octets; `BDAT` sends the four raw ones, which `SIZE 4` allows.
        (
            "CHUNKING measures SIZE on the raw message",
            plain(".x\r\n", &["bob@example.com"]),
            &["SIZE 4", "CHUNKING"],
            bdat(".x\r\n"),
        ),
        (
            "BDAT sends a message that does not end in CRLF",
            plain("no-crlf", &["bob@example.com"]),
            &["CHUNKING"],
            bdat("no-crlf"),
        ),
    ]
}

#[test]
fn each_submission_sends_the_envelope_its_server_allows() {
    let offered = |listed: &[&str], keyword: &str| {
        if listed.iter().any(|ext| ext.eq_ignore_ascii_case(keyword)) {
            Advertised::Offered
        } else {
            Advertised::Absent
        }
    };
    for (shows, mut session, listed, sent) in submissions() {
        let reply = replay(&mut session, &taken(listed, &sent)).unwrap();
        assert_eq!(reply.accepted.code, 250, "{shows}");
        let read = &reply.extensions;
        assert_eq!(read.smtputf8, offered(listed, "SMTPUTF8"), "{shows}");
        assert_eq!(read.dsn, offered(listed, "DSN"), "{shows}");
        assert_eq!(read.chunking, offered(listed, "CHUNKING"), "{shows}");
        assert_eq!(read.eight_bit_mime, offered(listed, "8BITMIME"), "{shows}");
    }
}

#[test]
fn a_utf8_local_part_is_unsupported_without_smtputf8_and_mail_from_is_not_sent() {
    // The transcript stops at EHLO. A MAIL FROM, or even AUTH, would still be
    // a write the trace does not expect, so the refusal is before either.
    let mut session = envelope("jörg@example.com", &["bob@example.com"], "hi\r\n", None);
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

/// `.x` stuffs to five octets, which `SIZE 4` refuses on `DATA` before anything is sent.
#[test]
fn without_chunking_size_is_measured_on_the_stuffed_message() {
    let mut session = plain(".x\r\n", &["bob@example.com"]);
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
