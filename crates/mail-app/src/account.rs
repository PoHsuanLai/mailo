//! Adding an account, and running one sync pass.
//!
//! This is where the pieces meet a real server, so it is also where the honest boundaries are:
//! a password is read from the environment rather than invented, and an OAuth account says what
//! it still needs rather than pretending to be configured.

use mail_domain::*;
use mail_runtime::{KeyringSecrets, Loopback, Secrets};
use mail_store::SqliteStore;
use std::fmt::Write as _;

/// Configure an account from its address.
///
/// The preset supplies hosts, ports and expected capabilities; the credential comes from the
/// environment and goes straight to the keyring. Nothing about a password is persisted in
/// SQLite, which is the whole reason `Credential` exists as a separate type.
pub fn add(
    store: &SqliteStore,
    address: &str,
    manual: Option<&mail_domain::presets::Manual>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    // Normalised once, here, and used for the preset, the stored plan and the stored column
    // alike. `preset_for` deliberately keeps the address exactly as typed, and the accounts
    // table deliberately lowercases it — so without this the plan and the column disagree in
    // case, and anything deriving a login name from one gets a different answer than anything
    // deriving it from the other. A mail server that is case-sensitive about the local part
    // then rejects one of them with nothing to explain why.
    let address = address.to_lowercase();
    let preset = match manual {
        // Explicit servers win over the table. Someone who names a host means that host, even
        // for a domain a preset happens to cover.
        Some(manual) => mail_domain::presets::manual(&address, manual, now),
        None => match mail_domain::presets::preset_for(&address, now) {
            Some(preset) => preset,
            None => {
                return Err(format!(
                    "no preset for {address:?}. Either it is one of the known domains \
                     (gmail.com, googlemail.com, ntu.edu.tw), or name the servers:\n\n  \
                     mailo account add {address} --imap imap.example.com --smtp smtp.example.com\n\n\
                     Ports default to 993 and 465, both with implicit TLS. Add --login NAME if \
                     the server wants something other than the whole address."
                ));
            }
        },
    };

    let account = AccountId::generate();

    // The preset leaves `identities` empty on purpose: minting one needs an `IdentityId` and
    // an `AccountId`, which would make `preset_for` impure and invent an account id no row
    // matches. Creating the account is where both exist, so this is where the default identity
    // is built — and without it nothing can be sent, because a draft names the identity it is
    // from and `mail_mime::build` reads the `From` header out of it.
    let mut plan = preset.plan;
    let identity = Identity {
        id: IdentityId::generate(),
        account,
        from: Address {
            // No display name. Inventing one from the local part produces "B09901185", and a
            // name the user did not choose is worse than no name: it goes out on every message.
            name: None,
            email: address.clone(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    };
    plan.identities = vec![identity.clone()];
    let plan_json =
        serde_json::to_string(&plan).map_err(|e| format!("cannot encode the account plan: {e}"))?;
    let caps_json = serde_json::to_string(&preset.expected_caps)
        .map_err(|e| format!("cannot encode capabilities: {e}"))?;

    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite_params(&[
                &account.to_string(),
                &address,
                &plan_json,
                &now.to_rfc3339(),
            ]),
        )
        .map_err(|e| format!("cannot save the account: {e}"))?;
    store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                 is_default)
             VALUES (?1, ?2, NULL, ?3, NULL, NULL, ?4)",
            rusqlite_params(&[
                &identity.id.to_string(),
                &account.to_string(),
                &identity.from.email,
                &serde_json::to_string(&identity.default)
                    .map_err(|e| format!("cannot encode the identity: {e}"))?,
            ]),
        )
        .map_err(|e| format!("cannot save the identity: {e}"))?;
    store
        .connection()
        .execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite_params(&[&account.to_string(), &caps_json, &now.to_rfc3339()]),
        )
        .map_err(|e| format!("cannot save capabilities: {e}"))?;

    let mut out = format!("added {address} as {account}\n");
    match &plan.auth {
        AuthPlan::Password { username, sasl } => {
            let login = username.resolve(&address);
            match std::env::var("MAILO_PASSWORD") {
                Ok(password) if !password.is_empty() => {
                    KeyringSecrets
                        .put(
                            &SecretKey {
                                account,
                                purpose: SecretPurpose::IncomingPassword,
                            },
                            &Credential::Password(password.clone()),
                        )
                        .map_err(|e| format!("cannot save the password: {e}"))?;
                    KeyringSecrets
                        .put(
                            &SecretKey {
                                account,
                                purpose: SecretPurpose::OutgoingPassword,
                            },
                            &Credential::Password(password),
                        )
                        .map_err(|e| format!("cannot save the password: {e}"))?;
                    let _ = writeln!(out, "password stored in the keyring for login {login:?}");
                    // Said here rather than at the first sync, where it arrives as an
                    // authentication failure with nothing to say it was never going to work.
                    if let Some(why) =
                        incoming_host(&plan).and_then(mail_domain::presets::password_warning)
                    {
                        let _ = writeln!(out, "\nwarning: {why}");
                    }
                }
                _ => {
                    // Saying what is missing beats a half-configured account that fails later
                    // with a less obvious message.
                    let _ = writeln!(
                        out,
                        "no password stored. Re-run with MAILO_PASSWORD set; \
                         the login name will be {login:?} and the server offers {sasl:?}."
                    );
                }
            }
        }
        AuthPlan::OAuth { issuer, scopes } => match std::env::var("MAILO_OAUTH_CLIENT_ID") {
            Ok(client_id) if !client_id.is_empty() => {
                let credential = authorize(*issuer, &client_id, scopes, now)?;
                KeyringSecrets
                    .put(
                        &SecretKey {
                            account,
                            purpose: SecretPurpose::OAuthRefresh,
                        },
                        &credential,
                    )
                    .map_err(|e| format!("cannot save the token: {e}"))?;
                // Incoming and outgoing share one OAuth credential: the scopes cover IMAP and
                // SMTP together, and storing it twice would mean refreshing it twice.
                KeyringSecrets
                    .put(
                        &SecretKey {
                            account,
                            purpose: SecretPurpose::IncomingPassword,
                        },
                        &credential,
                    )
                    .map_err(|e| format!("cannot save the token: {e}"))?;
                let _ = writeln!(out, "signed in; token stored in the keyring");
            }
            _ => {
                // The client id is deployment configuration and cannot be shipped in a source
                // tree, so the honest thing is to say exactly what is missing and how to
                // supply it — not to look configured and fail at first connect.
                let _ = writeln!(
                    out,
                    "this account uses OAuth ({issuer:?}) and needs a client id.\n\
                     Register an installed application with the issuer, then re-run:\n\
                     \n  MAILO_OAUTH_CLIENT_ID=… mailo account add {address}\n\
                     \nScopes it will request: {scopes:?}"
                );
            }
        },
    }
    Ok(out)
}

/// Run the browser sign-in and return the resulting credential.
///
/// Blocking, and deliberately so: this is a one-shot setup command, the user is watching, and
/// there is nothing else for the process to do while they sign in.
fn authorize(
    issuer: OAuthIssuer,
    client_id: &str,
    scopes: &[String],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;

    runtime.block_on(async {
        // Bind first: the redirect URI has to name the port we actually got, and an installed
        // application has no fixed one.
        let listener = Loopback::bind().await.map_err(|e| e.to_string())?;
        let authorization =
            mail_runtime::oauth::begin(issuer, client_id, scopes, listener.redirect_uri())
                .map_err(|e| e.to_string())?;

        println!(
            "Open this in a browser to sign in:\n\n  {}\n",
            authorization.url
        );
        println!("Waiting for the redirect…");

        let code = listener
            .wait_for_code(&authorization.pending)
            .await
            .map_err(|e| e.to_string())?;
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| format!("cannot build an HTTP client: {e}"))?;
        authorization
            .pending
            .exchange(&code, &http, now)
            .await
            .map_err(|e| e.to_string())
    })
}

/// Accounts, with what each one still needs.
pub fn list(store: &SqliteStore) -> Result<String, String> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT id, address FROM accounts ORDER BY created_at")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;

    let mut out = String::new();
    for row in rows {
        let (id, address) = row.map_err(|e| e.to_string())?;
        let Ok(uuid) = id.parse() else { continue };
        let account = AccountId::from_uuid(uuid);
        let has_password = KeyringSecrets
            .get(&SecretKey {
                account,
                purpose: SecretPurpose::IncomingPassword,
            })
            .is_ok();
        let _ = writeln!(
            out,
            "{address:<28} {}",
            if has_password {
                "ready"
            } else {
                "no credential stored"
            }
        );
    }
    if out.is_empty() {
        out.push_str("no accounts. Add one with: mailo account add <address>\n");
    }
    Ok(out)
}

/// `rusqlite::params!` over a slice, so the call sites stay readable.
/// The host mail arrives from, whichever protocol that is.
fn incoming_host(plan: &AccountPlan) -> Option<&str> {
    match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => Some(host.as_str()),
    }
}

fn rusqlite_params<'a>(values: &'a [&'a str]) -> impl rusqlite::Params + 'a {
    rusqlite::params_from_iter(values.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn store() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        (store, dir)
    }

    #[test]
    fn an_unknown_domain_says_which_are_known() {
        let (store, _dir) = store();
        let err = add(&store, "someone@example.test", None, now()).unwrap_err();
        assert!(err.contains("gmail.com"), "{err}");
        assert!(err.contains("ntu.edu.tw"), "{err}");
    }

    #[test]
    fn an_oauth_account_without_a_client_id_says_how_to_supply_one() {
        // Rather than appearing configured and failing at first connect with something less
        // obvious. A client id cannot be shipped in the source tree, and that is worth saying.
        let (store, _dir) = store();
        // With no MAILO_OAUTH_CLIENT_ID set, which is the state anyone starts in. The client
        // id is deployment configuration and cannot be shipped in a source tree, so the useful
        // thing is the exact command to run once they have one.
        let out = add(&store, "someone@gmail.com", None, now()).unwrap();
        assert!(out.contains("client id"), "{out}");
        assert!(
            out.contains("MAILO_OAUTH_CLIENT_ID=… mailo account add someone@gmail.com"),
            "it should print the command to re-run: {out}"
        );
        assert!(
            out.contains("https://mail.google.com/"),
            "and the scopes: {out}"
        );
    }

    #[test]
    fn a_password_account_names_the_login_it_resolved() {
        // NTU logs in with the local part, not the address. Getting that wrong is a failed
        // authentication with no explanation, so the CLI says which name it will use.
        let (store, _dir) = store();
        let out = add(&store, "b09901185@ntu.edu.tw", None, now()).unwrap();
        assert!(out.contains("b09901185"), "{out}");
        assert!(
            !out.contains("b09901185@ntu.edu.tw\""),
            "the login is the local part: {out}"
        );
    }

    #[test]
    fn adding_an_account_persists_its_plan_and_capabilities() {
        let (store, _dir) = store();
        add(&store, "b09901185@ntu.edu.tw", None, now()).unwrap();
        let accounts: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        let caps: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM account_caps", [], |r| r.get(0))
            .unwrap();
        assert_eq!((accounts, caps), (1, 1));
    }

    #[test]
    fn listing_nothing_explains_how_to_add_one() {
        let (store, _dir) = store();
        assert!(list(&store).unwrap().contains("mailo account add"));
    }
}
