//! `account remove <address> [--yes]`: what it would take with it, and then, asked again with
//! `--yes`, taking it.
//!
//! Without `--yes` nothing is removed and the run fails, so a script that forgot it does not read
//! a removal into a success. The words are the window's sheet's, said for a terminal.

use super::{Consent, account_named};
use mail_core::AccountSecrets;
use mail_core::account::{
    Added, ClientRecord, GraphSetup, Listed, MicrosoftRoute, Outcome, Readiness,
};
use mail_core::{SqliteStore, Store};
use mail_domain::presets::PasswordWarning;
use mail_domain::{AccountPlan, Incoming, LeaveOnServer};
use porter_provider::Issuer;
use std::fmt::Write as _;
use std::path::Path;

/// Remove the account at `address`, or say what removing it would take. `config` is where the
/// offline setting is kept, when there is a config directory.
pub(super) fn remove(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    config: Option<&Path>,
    address: &str,
    consent: Consent,
) -> Result<String, String> {
    let id = account_named(store, address)?;
    match consent {
        Consent::Ask => {
            let held = store.offline(id).map_or(0, |offline| offline.messages);
            let incoming = plan_of(store, address).map(|plan| plan.incoming);
            Err(asking(address, held, incoming.as_ref()))
        }
        Consent::Given => {
            // Linked to accountd, an account Mail signed in itself is set aside and its sign-in is
            // in mailo's own store, which the linked secrets do not reach: forget it from there.
            let own = (store.granted_only() && store.held_accounts().contains(&id))
                .then(crate::edge::own_secrets);
            let secrets = own.as_deref().unwrap_or(secrets);
            let removed =
                crate::edge::block_on(mail_core::account::remove(store, secrets, id.clone()))
                    .map_err(|e| e.to_string())?;
            if let Some(config) = config {
                let _ = mail_core::offline::save(config, id, mail_core::offline::Keep::Bodies);
            }
            Ok(format!(
                "removed {} and {} of stored mail\n",
                removed.address,
                mail_core::attach::human_size(removed.freed.bytes)
            ))
        }
    }
}

fn plan_of(store: &SqliteStore, address: &str) -> Option<AccountPlan> {
    store
        .account_by_address(&address.to_lowercase())
        .ok()??
        .plan
        .ok()
}

/// What removing `address`, which holds `held` messages here, would take, and how to go on.
fn asking(address: &str, held: u64, incoming: Option<&Incoming>) -> String {
    let mail = match held {
        1 => "1 message".to_owned(),
        n => format!("{n} messages"),
    };
    let server = match incoming {
        Some(Incoming::Pop3 {
            leave: LeaveOnServer::DeleteAfterFetch,
            ..
        }) => format!(
            "This server deletes mail once it is downloaded, so these {mail} are the only copy \
             and will be gone for good."
        ),
        _ => "Mail on the server is not touched.".to_owned(),
    };
    format!(
        "removing {address} takes its {mail} on this computer, its folders and rules and its \
         saved sign-in. {server} Keys and certificates stay.\nNothing was removed. Run it again \
         with --yes to remove it."
    )
}

/// What the command line says when a sign-in needs a browser: the address to open, then that it
/// is waiting. Written to `out` so a test can read it.
fn print_signin(url: &str, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(out, "Open this in a browser to sign in:\n\n  {url}\n")?;
    writeln!(out, "Waiting for the redirect…")
}

/// [`print_signin`] to standard output, for `account add` to hand the sign-in.
pub(super) fn announce_sign_in(url: &str) {
    // As `println!` would, but a closed stdout is not worth a panic mid-sign-in.
    let _ = print_signin(url, &mut std::io::stdout().lock());
}

/// What `account add` prints once the account is stored.
pub(super) fn added(added: &Added) -> String {
    let Added {
        address,
        account,
        updated,
        outcome,
    } = added;
    let mut out = format!(
        "{} {address} as {account}\n",
        if *updated { "updated" } else { "added" }
    );
    match outcome {
        Outcome::PasswordStored {
            bearer,
            login,
            warning,
        } => {
            if *bearer {
                let _ = writeln!(out, "token stored in the keyring");
            } else {
                let _ = writeln!(out, "password stored in the keyring for login {login:?}");
            }
            if let Some(warning) = warning {
                let _ = writeln!(out, "\nwarning: {}", password_warning_words(*warning));
            }
        }
        Outcome::PasswordMissing { jmap, login, sasl } => {
            if *jmap {
                let _ = writeln!(
                    out,
                    "no password stored. Re-run with MAILO_PASSWORD set (the login name \
                     will be {login:?}), or with MAILO_JMAP_TOKEN for a bearer token."
                );
            } else {
                let _ = writeln!(
                    out,
                    "no password stored. Re-run with MAILO_PASSWORD set; \
                     the login name will be {login:?} and the server offers {sasl:?}."
                );
            }
        }
        Outcome::SignedIn { graph, client } => {
            match graph {
                Some(GraphSetup::ReadAndSend) => {
                    let _ = writeln!(out, "reading and sending go through Microsoft Graph");
                }
                Some(GraphSetup::SendOnly) => {
                    let _ = writeln!(out, "sending goes through Microsoft Graph");
                }
                Some(GraphSetup::Refused(why)) => {
                    let _ = writeln!(
                        out,
                        "warning: signed in, but Graph would not issue a token for \
                         sending ({why}). Mail will be received; sending needs Graph's \
                         Mail.Send permission on the app registration, consented to."
                    );
                }
                None => {}
            }
            match client {
                ClientRecord::NotRecorded => {
                    let _ = writeln!(out, "signed in; token stored in the keyring");
                }
                ClientRecord::Recorded(path) => {
                    let _ = writeln!(
                        out,
                        "signed in; token stored in the keyring, client id in {}",
                        path.display()
                    );
                }
                ClientRecord::Failed(why) => {
                    let _ = writeln!(
                        out,
                        "signed in; token stored in the keyring\n\
                         warning: could not record the client id ({why}), so renewing this \
                         sign-in will need MAILO_OAUTH_CLIENT_ID set again"
                    );
                }
            }
        }
        Outcome::NeedsClientId {
            issuer,
            scopes,
            route,
        } => {
            let flags = match route {
                MicrosoftRoute::ReceiveThroughGraph => " --microsoft --receive graph",
                MicrosoftRoute::SendThroughGraph => " --microsoft --send graph",
                MicrosoftRoute::Microsoft => " --microsoft",
                MicrosoftRoute::Other => "",
            };
            let _ = writeln!(
                out,
                "this account uses OAuth ({issuer:?}) and needs a client id.\n\
                 \n{where}\n\
                 \nThen re-run:\n\
                 \n  MAILO_OAUTH_CLIENT_ID=… {secret}mailo account add {address}{flags}\n\
                 \nThey are recorded after the first sign-in, so the variables are needed \
                 once.\n\
                 \nScopes it will request: {scopes:?}",
                // Named, because two bare `{}` fill in source order and these two read
                // perfectly plausibly the wrong way round.
                where = where_to_get_one(*issuer),
                secret = if matches!(issuer, Issuer::Google) {
                    "MAILO_OAUTH_CLIENT_SECRET=… "
                } else {
                    ""
                },
            );
        }
    }
    out
}

/// `account list`: each account and what it still needs.
pub(super) fn listed(accounts: &[Listed]) -> String {
    let mut out = String::new();
    for Listed {
        address,
        state,
        syncs,
    } in accounts
    {
        let waiting = match state {
            Readiness::Local => {
                let _ = writeln!(out, "{address:<28} kept on this computer; nothing to sync");
                continue;
            }
            Readiness::Ready => "ready",
            Readiness::NotSignedIn => "not signed in",
            Readiness::ServiceUnreachable => "the desktop's account service is not reachable",
            Readiness::NoCredential => "no credential stored",
        };
        let _ = writeln!(out, "{address:<28} {waiting}");
        if let Some(paths) = syncs {
            let _ = writeln!(out, "{:<28} syncs {}", "", paths.join(", "));
        }
    }
    if out.is_empty() {
        out.push_str("no accounts. Add one with: mailo account add <address>\n");
    }
    out
}

/// Where an installed-application client id comes from, per issuer.
///
/// Named rather than left as "register an installed application with the issuer", which is a
/// research task standing between someone and their own mail. A client id is the one thing this
/// program cannot supply — it is registered against the user's account with the issuer, and
/// shipping one in a source tree would mean every user of this client shared an identity and a
/// quota.
///
/// Deliberately names the durable things — the product, the credential type, the consent
/// requirement — and not a path through a menu, because console navigation is rewritten far more
/// often than any of those.
fn where_to_get_one(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => concat!(
            "Create one in the Google Cloud console (console.cloud.google.com) as an OAuth ",
            "client ID of application type \"Desktop app\", and download its JSON. While the ",
            "consent screen is still in Testing, the address above has to be listed as a test ",
            "user or the sign-in is refused — that is the step most people miss.\n",
            "\nGoogle issues a client *secret* with that client and will not exchange a code ",
            "without it, PKCE or no PKCE, so set MAILO_OAUTH_CLIENT_SECRET as well. Both are ",
            "in the downloaded JSON, as client_id and client_secret."
        ),
        Issuer::Microsoft => concat!(
            "Register an application in the Microsoft Entra admin centre (entra.microsoft.com) ",
            "under App registrations, with a redirect URI of type \"Public client/native\". A ",
            "managed tenant may also require an administrator to consent to the scopes below ",
            "before any sign-in succeeds."
        ),
        // porter names more issuers than mailo reads mail from, and nothing here creates an
        // account that signs in with one.
        _ => "mailo has no mail sign-in through this issuer.",
    }
}

/// What to tell someone about to store a password where the host will not take one.
///
/// Advice, not a refusal. A tenant may have re-enabled something, an app password may exist, and
/// the user knows their own account better than a table does, so this explains and proceeds.
fn password_warning_words(warning: PasswordWarning) -> &'static str {
    match warning {
        PasswordWarning::Microsoft365 => {
            "Microsoft 365 turned off password authentication for IMAP, POP and SMTP, so a \
             password will be rejected however it is stored. These mailboxes need OAuth, which \
             is queued in plan.md and not written yet."
        }
        PasswordWarning::Google => {
            "Google stopped accepting account passwords for IMAP and SMTP. An App Password (which \
             needs two-factor authentication switched on) still works here; the account's own \
             password will not."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{asking, remove};
    use crate::cli::Consent;
    use mail_core::SqliteStore;
    use mail_domain::{Incoming, LeaveOnServer, Tls, presets};
    use porter_secrets::MemorySecrets;

    const ADDRESS: &str = "me@nowhere.example";

    fn accounts(store: &SqliteStore) -> i64 {
        mail_store::testing::count(store, "accounts")
    }

    #[test]
    fn without_yes_nothing_is_removed_and_with_it_the_account_goes() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let manual = presets::Manual {
            imap_host: "imap.nowhere.example".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.nowhere.example".to_owned(),
            smtp_port: 465,
            login: None,
        };
        let preset = presets::manual(ADDRESS, &manual, chrono::Utc::now());
        mail_store::testing::seed_account_plan(
            &store,
            mail_domain::id::new_account_id(),
            ADDRESS,
            &preset.plan,
            None,
        );
        let secrets = MemorySecrets::default();

        let asked = remove(&store, &secrets, None, ADDRESS, Consent::Ask).unwrap_err();
        assert!(asked.contains("Nothing was removed"), "{asked}");
        assert!(asked.contains("its 0 messages"), "{asked}");
        assert_eq!(accounts(&store), 1);

        let said = remove(&store, &secrets, None, ADDRESS, Consent::Given).unwrap();
        assert!(said.starts_with("removed me@nowhere.example"), "{said}");
        assert_eq!(accounts(&store), 0);

        let again = remove(&store, &secrets, None, ADDRESS, Consent::Given).unwrap_err();
        assert!(again.contains("no account for"), "{again}");
    }

    #[test]
    fn asking_names_the_mail_and_whether_the_server_keeps_a_copy() {
        let pop3 = |leave| Incoming::Pop3 {
            host: "pop.example.test".to_owned(),
            port: 995,
            tls: Tls::Implicit,
            leave,
        };
        // (held, incoming, words it has, words it lacks)
        let cases = [
            (1, None, "its 1 message on this computer", "only copy"),
            (
                12,
                Some(pop3(LeaveOnServer::Keep)),
                "not touched",
                "only copy",
            ),
            (
                12,
                Some(pop3(LeaveOnServer::DeleteAfterFetch)),
                "these 12 messages are the only copy",
                "not touched",
            ),
        ];
        for (held, incoming, has, lacks) in cases {
            let said = asking("me@example.test", held, incoming.as_ref());
            assert!(said.contains(has), "{said}");
            assert!(!said.contains(lacks), "{said}");
            assert!(said.contains("Nothing was removed"), "{said}");
            assert!(said.contains("--yes"), "{said}");
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::{added, listed, print_signin};
    use mail_core::account::{
        Added, ClientRecord, GraphSetup, Listed, MicrosoftRoute, Outcome, Readiness,
    };
    use mail_domain::SaslMech;
    use porter_provider::Issuer;

    /// The command line's sign-in prompt is byte for byte what `authorize` printed itself before
    /// the address went through `on_url`: scripts that read it keep working.
    #[test]
    fn the_command_line_prints_the_sign_in_address_as_it_always_has() {
        let url = "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";
        let mut out = Vec::new();
        print_signin(url, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Open this in a browser to sign in:\n\n  \
             https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz\n\n\
             Waiting for the redirect…\n"
        );
    }

    fn done(address: &str, updated: bool, outcome: Outcome) -> Added {
        Added {
            address: address.to_owned(),
            account: mail_domain::id::new_account_id(),
            updated,
            outcome,
        }
    }

    #[test]
    fn re_running_says_updated_and_names_the_address() {
        let said = added(&done(
            "someone@example.test",
            true,
            Outcome::PasswordMissing {
                jmap: false,
                login: "someone@example.test".to_owned(),
                sasl: vec![SaslMech::Plain],
            },
        ));
        assert!(
            said.starts_with("updated someone@example.test as "),
            "{said}"
        );
        assert!(!said.contains("UNIQUE constraint"), "{said}");
    }

    #[test]
    fn an_oauth_account_without_a_client_id_says_how_to_supply_one() {
        // Rather than appearing configured and failing at first connect with something less
        // obvious. A client id cannot be shipped in the source tree, and that is worth saying.
        let said = added(&done(
            "someone@gmail.com",
            false,
            Outcome::NeedsClientId {
                issuer: Issuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
                route: MicrosoftRoute::Other,
            },
        ));
        assert!(said.contains("client id"), "{said}");
        assert!(
            said.contains(
                "MAILO_OAUTH_CLIENT_ID=… MAILO_OAUTH_CLIENT_SECRET=… \
                 mailo account add someone@gmail.com"
            ),
            "it should print the command to re-run: {said}"
        );
        // Google issues a secret with every Desktop-app client and refuses the exchange without
        // it. Naming only the client id is what sent the first real sign-in into
        // `invalid_request: client_secret is missing`.
        assert!(
            said.contains("client_secret"),
            "it should say where the secret comes from: {said}"
        );
        assert!(
            said.contains("https://mail.google.com/"),
            "and the scopes: {said}"
        );
    }

    #[test]
    fn a_microsoft_account_keeps_its_flags_in_the_command_to_re_run() {
        for (route, flags) in [
            (MicrosoftRoute::Microsoft, " --microsoft\n"),
            (
                MicrosoftRoute::SendThroughGraph,
                " --microsoft --send graph\n",
            ),
            (
                MicrosoftRoute::ReceiveThroughGraph,
                " --microsoft --receive graph\n",
            ),
        ] {
            let said = added(&done(
                "ada@example.test",
                false,
                Outcome::NeedsClientId {
                    issuer: Issuer::Microsoft,
                    scopes: Vec::new(),
                    route,
                },
            ));
            assert!(
                said.contains(&format!("mailo account add ada@example.test{flags}")),
                "{said}"
            );
            assert!(!said.contains("MAILO_OAUTH_CLIENT_SECRET"), "{said}");
        }
    }

    #[test]
    fn a_password_account_names_the_login_it_resolved() {
        // Some servers log in with a student or staff number, not the address. Getting that
        // wrong is a failed authentication with no explanation, so the CLI says which name it
        // will use.
        let said = added(&done(
            "s1234567@example.edu",
            false,
            Outcome::PasswordMissing {
                jmap: false,
                login: "s1234567".to_owned(),
                sasl: vec![SaslMech::Plain],
            },
        ));
        assert!(
            said.contains("the login name will be \"s1234567\""),
            "{said}"
        );
        assert!(!said.contains("s1234567@example.edu\""), "{said}");
    }

    #[test]
    fn a_signed_in_account_says_what_became_of_graph_and_the_client_id() {
        let said = added(&done(
            "ada@example.test",
            false,
            Outcome::SignedIn {
                graph: Some(GraphSetup::ReadAndSend),
                client: ClientRecord::Recorded("/home/ada/oauth.json".into()),
            },
        ));
        assert!(
            said.ends_with(
                "reading and sending go through Microsoft Graph\n\
                 signed in; token stored in the keyring, client id in /home/ada/oauth.json\n"
            ),
            "{said}"
        );
        let said = added(&done(
            "ada@example.test",
            false,
            Outcome::SignedIn {
                graph: None,
                client: ClientRecord::NotRecorded,
            },
        ));
        assert!(
            said.ends_with("signed in; token stored in the keyring\n"),
            "{said}"
        );
    }

    #[test]
    fn listing_nothing_explains_how_to_add_one() {
        assert!(listed(&[]).contains("mailo account add"));
    }

    #[test]
    fn the_listing_says_what_each_account_waits_for() {
        let account = |state| Listed {
            address: "ada@example.test".to_owned(),
            state,
            syncs: None,
        };
        // An OAuth account must not be told a credential is missing: that reads as "find a
        // password", which is the one thing that will not work.
        assert!(listed(&[account(Readiness::NotSignedIn)]).contains("not signed in"));
        assert!(listed(&[account(Readiness::NoCredential)]).contains("no credential stored"));
        assert_eq!(
            listed(&[Listed {
                syncs: Some(vec!["INBOX".to_owned(), "Sent".to_owned()]),
                ..account(Readiness::Ready)
            }]),
            format!(
                "{:<28} ready\n{:<28} syncs INBOX, Sent\n",
                "ada@example.test", ""
            )
        );
        assert_eq!(
            listed(&[account(Readiness::Local)]),
            format!(
                "{:<28} kept on this computer; nothing to sync\n",
                "ada@example.test"
            )
        );
    }
}
