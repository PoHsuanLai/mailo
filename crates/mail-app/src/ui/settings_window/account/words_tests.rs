use super::{asking, refused, settings, storage};
use crate::ui::copy::status::account::{amount, storage_nearly_full, storage_used};
use mail_core::Storage;
use mail_core::account::RemoveError;
use mail_domain::{
    AccountPlan, Address, AuthPlan, HttpAuth, Identity, Incoming, LeaveOnServer, Outgoing,
    SaslMech, Tls, Username,
};
use porter_provider::Issuer;

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

/// Each row as `label: value`, so a case reads as the page does.
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
                "Sending: SMTP, smtp.example.com:587, not encrypted",
                "Signs in: With a password, as s1234567",
            ],
        ),
        (
            plan(
                Incoming::Graph,
                Outgoing::Graph,
                AuthPlan::OAuth {
                    issuer: Issuer::Microsoft,
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

/// The addresses an account sends as are not among its servers: they are the Sending as group's,
/// which edits them (`sends_as`), and the plan's copy of them changes nothing here.
#[test]
fn the_addresses_it_sends_as_are_not_server_lines() {
    let identity = |name: Option<&str>, email: &str| Identity {
        from: Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        },
        ..mail_domain::testing::identity()
    };
    let bare = plan(
        Incoming::Graph,
        Outgoing::Graph,
        AuthPlan::OAuth {
            issuer: Issuer::Google,
            scopes: Vec::new(),
        },
    );
    let mut sending = bare.clone();
    sending.identities = vec![
        identity(Some("Ada Lovelace"), ADDRESS),
        identity(None, "ada@work.example"),
    ];
    assert_eq!(said(&sending), said(&bare));
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
        (pop3(LeaveOnServer::Keep), 3, "isn’t deleted", "only copy"),
        (
            pop3(LeaveOnServer::DeleteAfterFetch),
            3,
            "these 3 messages are the only copy",
            "isn’t deleted",
        ),
    ];
    for (incoming, held, has, lacks) in cases {
        let asked = asking(ADDRESS, held, &incoming);
        assert_eq!(asked.title, "Remove ada@example.com?");
        assert_eq!(asked.confirm, crate::ui::copy::controls::REMOVE);
        assert!(asked.body.contains(has), "{incoming:?}: {}", asked.body);
        assert!(!asked.body.contains(lacks), "{incoming:?}: {}", asked.body);
        assert!(
            asked.body.contains("Keys and certificates aren’t removed"),
            "{}",
            asked.body
        );
    }
}

#[test]
fn a_refusal_says_the_account_was_not_removed_or_already_was() {
    let cases = [
        (RemoveError::Unknown, "already gone"),
        (RemoveError::Local, "can’t be removed"),
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

#[test]
fn an_amount_is_said_in_the_largest_unit_it_fills() {
    let cases: [(u64, &str); 17] = [
        (0, "0 B"),
        (512, "512 B"),
        (1023, "1023 B"),
        (1024, "1 kB"),
        (1536, "1.5 kB"),
        (300 << 20, "300 MB"),
        // Rounds to 1023.9 kB, and one byte more to a whole MB, never "1024.0 kB".
        (1_048_524, "1023.9 kB"),
        (1_048_525, "1 MB"),
        ((1 << 20) - 1, "1 MB"),
        (1 << 20, "1 MB"),
        (1288 << 20, "1.3 GB"),
        ((1 << 30) - 1, "1 GB"),
        (15 << 30, "15 GB"),
        ((1 << 40) - 1, "1 TB"),
        (2 << 40, "2 TB"),
        (1500 << 40, "1500 TB"),
        (u64::MAX, "16777216 TB"),
    ];
    for (bytes, said) in cases {
        assert_eq!(amount(bytes), said, "{bytes}");
    }
}

#[test]
fn storage_says_what_is_used_of_the_limit_and_when_it_is_nearly_full() {
    let cases = [
        (
            Storage {
                used: 1 << 30,
                limit: 15 << 30,
            },
            false,
        ),
        (
            Storage {
                used: 14 << 30,
                limit: 15 << 30,
            },
            true,
        ),
        (
            Storage {
                used: 16 << 30,
                limit: 15 << 30,
            },
            true,
        ),
    ];
    for (found, nearly_full) in cases {
        let (used, limit) = (amount(found.used), amount(found.limit));
        let said = if nearly_full {
            storage_nearly_full(&used, &limit)
        } else {
            storage_used(&used, &limit)
        };
        let line = storage(&found);
        assert_eq!(line.label, crate::ui::copy::fields::account::STORAGE);
        assert_eq!(line.value, said, "{found:?}");
    }
}
