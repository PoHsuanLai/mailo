//! What the client says when a server refuses it.
//!
//! The failures these cover all look like "wrong password" from the client's side, and none of
//! them is. A user who believes the client will retype a correct password until their patience
//! runs out, which is the actual cost of an unexplained refusal.

use mail_proto::machine::{ProtoError, Refusal};
use mail_proto::{Diagnosis, diagnose, diagnose_text};

/// `(name, refusal text, what it is diagnosed as, or None for no diagnosis)`.
const CASES: &[(&str, &str, Option<Diagnosis>)] = &[
    // The one a work or school Microsoft 365 mailbox hits, and the reason it matters: it is a
    // tenant setting. No amount of retyping, re-adding the account or re-registering an OAuth
    // application changes it — an administrator does. The point is that the credential is fine.
    (
        "a tenant with SMTP AUTH disabled is named as such",
        "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is \
         disabled for the tenant",
        Some(Diagnosis::TenantSmtpAuthOff),
    ),
    // Different remedies, and choosing the wrong one wastes an afternoon: an App Password is
    // made in minutes, and "your password is wrong" sends someone to reset a correct one.
    (
        "google two-factor asks for an App Password",
        "534-5.7.9 Application-specific password required",
        Some(Diagnosis::AppPasswordNeeded),
    ),
    (
        "google's rejected password points at an App Password",
        "535-5.7.8 Username and Password not accepted",
        Some(Diagnosis::GoogleRejectedCredential),
    ),
    (
        "a protocol switched off for the mailbox is not a credential problem",
        "NO IMAP4 access is disabled for this mailbox",
        Some(Diagnosis::ProtocolSwitchedOff),
    ),
    // The common case, and the safe one: the server's own words are shown unchanged. A
    // confident wrong explanation is worse than the raw text it replaced.
    (
        "unrecognised: a generic invalid login",
        "535 5.7.0 Invalid login or password",
        None,
    ),
    (
        "unrecognised: IMAP AUTHENTICATIONFAILED",
        "NO [AUTHENTICATIONFAILED] Authentication failed.",
        None,
    ),
    ("unrecognised: POP3 -ERR", "-ERR authorization failed", None),
    ("unrecognised: 421", "421 Service not available", None),
    ("unrecognised: empty", "", None),
    // Servers are not consistent about case.
    (
        "matching ignores case: SMTP",
        "535 5.7.139 SMTPCLIENTAUTHENTICATION IS DISABLED",
        Some(Diagnosis::TenantSmtpAuthOff),
    ),
    (
        "matching ignores case: IMAP",
        "no imap4 access is disabled for this mailbox",
        Some(Diagnosis::ProtocolSwitchedOff),
    ),
    // K1: `contains("5.7.139")` matches "5.7.1399". The mistake has appeared three times in
    // this codebase in three different disguises, so each shape is its own row.
    (
        "K1 a longer code is not read as a shorter one: 5.7.1399",
        "5.7.1399",
        None,
    ),
    (
        "K1 a longer code is not read as a shorter one: 15.7.139",
        "535 15.7.139 something else",
        None,
    ),
    (
        "K1 a longer code is not read as a shorter one: 5.7.89",
        "5.7.89",
        None,
    ),
    // The genuine code, in the shapes providers actually send.
    (
        "K1 the genuine 5.7.139 with its text",
        "535 5.7.139 Authentication unsuccessful",
        Some(Diagnosis::TenantSmtpAuthOff),
    ),
    (
        "K1 the genuine 5.7.139 alone",
        "535 5.7.139",
        Some(Diagnosis::TenantSmtpAuthOff),
    ),
    (
        "K1 the genuine 5.7.139 at the start",
        "5.7.139 at the start",
        Some(Diagnosis::TenantSmtpAuthOff),
    ),
];

#[test]
fn refusal_texts_are_diagnosed_or_left_alone() {
    for (name, text, want) in CASES {
        assert_eq!(
            diagnose_text(text),
            *want,
            "{name}: {text:?} is diagnosed wrongly"
        );
    }
}

#[test]
fn it_reads_the_error_as_well_as_the_text() {
    let err = ProtoError::AuthRejected(
        "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled".to_owned(),
    );
    assert_eq!(diagnose(&err), Some(Diagnosis::TenantSmtpAuthOff));

    let refused = ProtoError::Refused {
        kind: Refusal::Permanent,
        text: "534-5.7.9 Application-specific password required".to_owned(),
    };
    assert_eq!(diagnose(&refused), Some(Diagnosis::AppPasswordNeeded));
}

#[test]
fn an_error_that_is_not_a_refusal_is_not_diagnosed() {
    // A parse failure or a dropped connection has nothing to do with credentials, and offering
    // credential advice for one would send the user in exactly the wrong direction.
    assert_eq!(diagnose(&ProtoError::UnexpectedEof), None);
    assert_eq!(diagnose(&ProtoError::Malformed("5.7.139".to_owned())), None);
}

/// Honouring `LOGINDISABLED`, which outlook.office365.com really does advertise.
mod login_disabled {
    use mail_domain::SaslMech;
    use mail_proto::machine::{IoReady, Machine, Progress};
    use mail_proto::{ImapAuth, ImapCommand, ImapSession, has_capability};
    use porter_core::{Credential, SecretText};

    /// The exact capability line outlook.office365.com sent on 2026-09-22.
    const EXCHANGE: &[u8] = b"* CAPABILITY IMAP4 IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED SASL-IR \
UIDPLUS MOVE ID UNSELECT CHILDREN IDLE NAMESPACE LITERAL+\r\n";

    fn session(commands: Vec<ImapCommand>) -> ImapSession {
        ImapSession::new(
            ImapAuth {
                username: "me@example.test".to_owned(),
                credential: Credential::Password(SecretText::new("pw".to_owned())),
                sasl: vec![SaslMech::Plain],
            },
            commands,
        )
        .unwrap()
    }

    #[test]
    fn a_password_is_not_sent_to_a_server_that_said_it_will_refuse_one() {
        // RFC 3501 §6.2.3 says a client MUST NOT issue LOGIN when this is advertised. The cost
        // of ignoring it is not only conformance: the password goes to something that already
        // said no, and the rejection then reads as a wrong credential.
        let mut s = session(vec![ImapCommand::Capability, ImapCommand::Login]);
        let _ = s.start();
        let _ = s.feed(IoReady::Bytes(b"* OK ready\r\n".to_vec()));
        let _ = s.feed(IoReady::Bytes(EXCHANGE.to_vec()));
        let progress = s.feed(IoReady::Bytes(b"a001 OK done\r\n".to_vec()));

        match progress {
            Progress::Failed(e) => {
                let text = format!("{e}");
                assert!(text.contains("LOGINDISABLED"), "{text}");
                assert!(text.contains("not sent"), "{text}");
                // It is read by a person. A `\`-continuation in the literal was collapsed by
                // `cargo fmt` into the middle of the sentence, so this shipped with a
                // twenty-six-space gap in it and nothing noticed.
                assert!(
                    !text.contains("  "),
                    "a run of spaces in a message: {text:?}"
                );
            }
            other => panic!("LOGIN was attempted anyway: {other:?}"),
        }
    }

    #[test]
    fn a_server_that_permits_login_still_gets_one() {
        // The check must not refuse every password account; a stock Dovecot advertises no such
        // capability, and its accounts work.
        let mut s = session(vec![ImapCommand::Capability, ImapCommand::Login]);
        let _ = s.start();
        let _ = s.feed(IoReady::Bytes(b"* OK ready\r\n".to_vec()));
        let _ = s.feed(IoReady::Bytes(
            b"* CAPABILITY IMAP4rev1 UIDPLUS IDLE\r\n".to_vec(),
        ));
        let progress = s.feed(IoReady::Bytes(b"a001 OK done\r\n".to_vec()));
        assert!(
            matches!(progress, Progress::Need(_)),
            "a permissive server was refused: {progress:?}"
        );
    }

    #[test]
    fn capabilities_match_as_whole_atoms_not_substrings() {
        // The fourth appearance of one mistake (FINDINGS F67, F69). `imap-proto` renders atoms
        // with Debug, so an entry is `Atom("MOVE")` — and `contains("MOVE")` is also true of
        // `Atom("REMOVE")`, `contains("UID")` of `Atom("UIDPLUS")`.
        let caps: Vec<String> = [
            "Atom(\"LOGINDISABLED\")",
            "Auth(\"XOAUTH2\")",
            "Imap4rev1",
            "Atom(\"UIDPLUS\")",
            "Atom(\"MOVE\")",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

        assert!(has_capability(&caps, "LOGINDISABLED"));
        assert!(has_capability(&caps, "logindisabled"), "case-insensitive");
        assert!(has_capability(&caps, "XOAUTH2"));
        assert!(has_capability(&caps, "MOVE"));
        assert!(has_capability(&caps, "IMAP4REV1"));

        assert!(!has_capability(&caps, "UID"), "UIDPLUS is not UID");
        assert!(!has_capability(&caps, "OVE"), "MOVE is not OVE");
        assert!(!has_capability(&caps, "CONDSTORE"));
        assert!(!has_capability(&caps, ""));
    }
}
