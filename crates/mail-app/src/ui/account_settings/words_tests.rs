use super::{asking, refused, settings};
use mail_core::account::RemoveError;
use mail_domain::{
    AccountId, AccountPlan, Address, AuthPlan, HttpAuth, Identity, IdentityId, Incoming, IsDefault,
    LeaveOnServer, OAuthIssuer, Outgoing, SaslMech, Tls, Username,
};

const ADDRESS: &str = "ada@example.com";

fn password(username: Username) -> AuthPlan {
    AuthPlan::Password {
        username,
        sasl: vec![SaslMech::Plain],
    }
}

fn plan(incoming: Incoming, outgoing: Outgoing, auth: AuthPlan) -> AccountPlan {
    AccountPlan {
        address: ADDRESS.to_owned(),
        incoming,
        outgoing,
        auth,
        identities: Vec::new(),
    }
}

fn smtp(tls: Tls) -> Outgoing {
    Outgoing::Smtp {
        host: "smtp.example.com".to_owned(),
        port: 587,
        tls,
    }
}

fn pop3(leave: LeaveOnServer) -> Incoming {
    Incoming::Pop3 {
        host: "pop.example.com".to_owned(),
        port: 995,
        tls: Tls::Implicit,
        leave,
    }
}

/// Each row as `label: value`, so a case reads as the sheet does.
fn said(plan: &AccountPlan) -> Vec<String> {
    settings(plan)
        .into_iter()
        .map(|line| format!("{}: {}", line.label, line.value))
        .collect()
}

#[test]
fn each_kind_of_account_lists_its_servers_and_how_it_signs_in() {
    let cases: Vec<(AccountPlan, &[&str])> = vec![
        (
            plan(
                Incoming::Imap {
                    host: "imap.example.com".to_owned(),
                    port: 993,
                    tls: Tls::Implicit,
                },
                smtp(Tls::StartTlsRequired),
                password(Username::LocalPart),
            ),
            &[
                "Receiving: IMAP, imap.example.com:993, TLS",
                "Sending: SMTP, smtp.example.com:587, STARTTLS",
                "Signs in: With a password, as ada",
            ],
        ),
        (
            plan(
                pop3(LeaveOnServer::DeleteAfterFetch),
                smtp(Tls::Plaintext),
                password(Username::Literal("s1234567".to_owned())),
            ),
            &[
                "Receiving: POP3, pop.example.com:995, TLS",
                "Downloaded mail: Deleted from the server",
                "Sending: SMTP, smtp.example.com:587, not encrypted",
                "Signs in: With a password, as s1234567",
            ],
        ),
        (
            plan(
                Incoming::Graph,
                Outgoing::Graph,
                AuthPlan::OAuth {
                    issuer: OAuthIssuer::Microsoft,
                    scopes: Vec::new(),
                },
            ),
            &[
                "Receiving: Microsoft Graph",
                "Sending: Microsoft Graph",
                "Signs in: With Microsoft, in the browser",
            ],
        ),
        (
            plan(
                Incoming::Jmap {
                    session: "https://jmap.example.com/.well-known/jmap".to_owned(),
                    auth: HttpAuth::Bearer,
                },
                Outgoing::Jmap,
                password(Username::SameAsAddress),
            ),
            &[
                "Receiving: JMAP, https://jmap.example.com/.well-known/jmap",
                "Sending: JMAP, on the same server",
                "Signs in: With a token, as ada@example.com",
            ],
        ),
    ];
    for (plan, expect) in cases {
        assert_eq!(said(&plan), *expect, "{plan:?}");
    }
}

#[test]
fn the_addresses_it_sends_as_follow_with_their_names() {
    let identity = |name: Option<&str>, email: &str| Identity {
        id: IdentityId::generate(),
        account: AccountId::generate(),
        from: Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    };
    let mut plan = plan(
        Incoming::Graph,
        Outgoing::Graph,
        AuthPlan::OAuth {
            issuer: OAuthIssuer::Google,
            scopes: Vec::new(),
        },
    );
    plan.identities = vec![
        identity(Some(" Ada Lovelace "), ADDRESS),
        identity(Some(""), "ada@work.example"),
        identity(None, "a@example.org"),
    ];
    assert_eq!(
        said(&plan).last().map(String::as_str),
        Some("Sends as: Ada Lovelace <ada@example.com>, ada@work.example, a@example.org")
    );
}

#[test]
fn the_confirmation_counts_the_mail_and_says_when_it_is_the_only_copy() {
    // (incoming, held, words the body must have, words it must not)
    let imap = Incoming::Imap {
        host: "imap.example.com".to_owned(),
        port: 993,
        tls: Tls::Implicit,
    };
    let cases = [
        (
            imap.clone(),
            1,
            "Its 1 message on this computer",
            "only copy",
        ),
        (imap, 1204, "Its 1\u{a0}204 messages", "only copy"),
        (pop3(LeaveOnServer::Keep), 3, "not touched", "only copy"),
        (
            pop3(LeaveOnServer::DeleteAfterFetch),
            3,
            "these 3 messages are the only copy",
            "not touched",
        ),
    ];
    for (incoming, held, has, lacks) in cases {
        let asked = asking(ADDRESS, held, &incoming);
        assert_eq!(asked.title, "Remove ada@example.com?");
        assert_eq!(asked.confirm, "Remove Account");
        assert!(asked.body.contains(has), "{incoming:?}: {}", asked.body);
        assert!(!asked.body.contains(lacks), "{incoming:?}: {}", asked.body);
        assert!(
            asked.body.contains("Keys and certificates stay"),
            "{}",
            asked.body
        );
    }
}

#[test]
fn a_refusal_says_that_nothing_was_removed_or_that_it_already_was() {
    let cases = [
        (RemoveError::Unknown, "already removed"),
        (RemoveError::Local, "cannot be removed"),
        (
            RemoveError::Keyring("locked".to_owned()),
            "nothing was removed",
        ),
        (
            RemoveError::Store("disk full".to_owned()),
            "Nothing was removed: disk full",
        ),
    ];
    for (error, has) in cases {
        let said = refused(&error);
        assert!(said.contains(has), "{error:?}: {said}");
    }
}
