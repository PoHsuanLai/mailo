//! What the client says when a server refuses it.
//!
//! The failures these cover all look like "wrong password" from the client's side, and none of
//! them is. A user who believes the client will retype a correct password until their patience
//! runs out, which is the actual cost of an unexplained refusal.

use mail_proto::machine::{ProtoError, Refusal};
use mail_proto::{explain, explain_text};

#[test]
fn a_tenant_with_smtp_auth_disabled_is_named_as_such() {
    // The one a work or school Microsoft 365 mailbox hits, and the reason it matters: it is a
    // tenant setting. No amount of retyping, re-adding the account or re-registering an OAuth
    // application changes it — an administrator does.
    let refusal = "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is \
                   disabled for the tenant";
    let why = explain_text(refusal).expect("5.7.139 is documented");
    assert!(why.contains("Set-CASMailbox"), "{why}");
    assert!(
        why.contains("Nothing about the password is wrong"),
        "the point of this is that the credential is fine: {why}"
    );
}

#[test]
fn google_two_factor_is_told_apart_from_a_bad_password() {
    // Different remedies, and choosing the wrong one wastes an afternoon: an App Password is
    // made in minutes, and "your password is wrong" sends someone to reset a correct one.
    let app_password = explain_text("534-5.7.9 Application-specific password required").unwrap();
    assert!(app_password.contains("App Password"), "{app_password}");

    let rejected =
        explain_text("535-5.7.8 Username and Password not accepted").expect("documented");
    assert!(rejected.contains("App Password"), "{rejected}");
}

#[test]
fn a_protocol_switched_off_for_the_mailbox_is_not_a_credential_problem() {
    let why = explain_text("NO IMAP4 access is disabled for this mailbox").expect("documented");
    assert!(why.contains("administrator"), "{why}");
}

#[test]
fn it_reads_the_error_as_well_as_the_text() {
    let err = ProtoError::AuthRejected(
        "535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled".to_owned(),
    );
    assert!(explain(&err).is_some());

    let refused = ProtoError::Refused {
        kind: Refusal::Permanent,
        text: "534-5.7.9 Application-specific password required".to_owned(),
    };
    assert!(explain(&refused).is_some());
}

#[test]
fn an_error_that_is_not_a_refusal_is_not_diagnosed() {
    // A parse failure or a dropped connection has nothing to do with credentials, and offering
    // credential advice for one would send the user in exactly the wrong direction.
    assert_eq!(explain(&ProtoError::UnexpectedEof), None);
    assert_eq!(explain(&ProtoError::Malformed("5.7.139".to_owned())), None);
}

#[test]
fn an_unrecognised_refusal_gets_no_invented_explanation() {
    // The common case, and the safe one: the server's own words are shown unchanged. A confident
    // wrong explanation is worse than the raw text it replaced.
    for text in [
        "535 5.7.0 Invalid login or password",
        "NO [AUTHENTICATIONFAILED] Authentication failed.",
        "-ERR authorization failed",
        "421 Service not available",
        "",
        "5.7.1399",
    ] {
        assert_eq!(
            explain_text(text),
            None,
            "invented an explanation for {text:?}"
        );
    }
}

#[test]
fn matching_is_case_insensitive_because_servers_are_not_consistent() {
    assert!(explain_text("535 5.7.139 SMTPCLIENTAUTHENTICATION IS DISABLED").is_some());
    assert!(explain_text("no imap4 access is disabled for this mailbox").is_some());
}

#[test]
fn a_longer_code_is_not_read_as_a_shorter_one() {
    // `contains("5.7.139")` matches "5.7.1399". Caught by this file's own unrecognised-refusal
    // case; kept as its own test because the mistake has now appeared three times in this
    // codebase in three different disguises.
    assert_eq!(explain_text("5.7.1399"), None);
    assert_eq!(explain_text("535 15.7.139 something else"), None);
    assert_eq!(explain_text("5.7.89"), None);
    // The genuine code, in the shapes providers actually send.
    assert!(explain_text("535 5.7.139 Authentication unsuccessful").is_some());
    assert!(explain_text("535 5.7.139").is_some());
    assert!(explain_text("5.7.139 at the start").is_some());
}

/// Honouring `LOGINDISABLED`, which outlook.office365.com really does advertise.
mod login_disabled {
    use mail_domain::{Credential, SaslMech};
    use mail_proto::machine::{IoReady, Machine, Progress};
    use mail_proto::{ImapAuth, ImapCommand, ImapSession, has_capability};

    /// The exact capability line outlook.office365.com sent on 2026-09-22.
    const EXCHANGE: &[u8] = b"* CAPABILITY IMAP4 IMAP4rev1 AUTH=XOAUTH2 LOGINDISABLED SASL-IR \
UIDPLUS MOVE ID UNSELECT CHILDREN IDLE NAMESPACE LITERAL+\r\n";

    fn session(commands: Vec<ImapCommand>) -> ImapSession {
        ImapSession::new(
            ImapAuth {
                username: "me@example.test".to_owned(),
                credential: Credential::Password("pw".to_owned()),
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
