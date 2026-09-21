//! Adding an account, and running one sync pass.
//!
//! This is where the pieces meet a real server, so it is also where the honest boundaries are:
//! a password is read from the environment rather than invented, and an OAuth account says what
//! it still needs rather than pretending to be configured.

use mail_domain::*;
use mail_runtime::{KeyringSecrets, Secrets};
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
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    let Some(preset) = mail_domain::presets::preset_for(address, now) else {
        return Err(format!(
            "no preset for {address:?}. Manual setup is not written yet — \
             the known domains are gmail.com, googlemail.com and ntu.edu.tw."
        ));
    };

    let account = AccountId::generate();
    let plan_json = serde_json::to_string(&preset.plan)
        .map_err(|e| format!("cannot encode the account plan: {e}"))?;
    let caps_json = serde_json::to_string(&preset.expected_caps)
        .map_err(|e| format!("cannot encode capabilities: {e}"))?;

    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite_params(&[
                &account.to_string(),
                &address.to_lowercase(),
                &plan_json,
                &now.to_rfc3339(),
            ]),
        )
        .map_err(|e| format!("cannot save the account: {e}"))?;
    store
        .connection()
        .execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite_params(&[&account.to_string(), &caps_json, &now.to_rfc3339()]),
        )
        .map_err(|e| format!("cannot save capabilities: {e}"))?;

    let mut out = format!("added {address} as {account}\n");
    match &preset.plan.auth {
        AuthPlan::Password { username, sasl } => {
            let login = username.resolve(address);
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
        AuthPlan::OAuth { issuer, scopes } => {
            let _ = writeln!(
                out,
                "this account uses OAuth ({issuer:?}) and needs a browser sign-in, \
                 which is not wired into the CLI yet.\n\
                 It also needs a client id: an installed-app credential registered with the \
                 issuer, which cannot be shipped in the source tree.\n\
                 Scopes: {scopes:?}"
            );
        }
    }
    Ok(out)
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
        let err = add(&store, "someone@example.test", now()).unwrap_err();
        assert!(err.contains("gmail.com"), "{err}");
        assert!(err.contains("ntu.edu.tw"), "{err}");
    }

    #[test]
    fn an_oauth_account_says_what_it_still_needs() {
        // Rather than appearing configured and failing at first connect with something less
        // obvious. A client id cannot be shipped in the source tree, and that is worth saying.
        let (store, _dir) = store();
        let out = add(&store, "someone@gmail.com", now()).unwrap();
        assert!(out.contains("OAuth"), "{out}");
        assert!(out.contains("client id"), "{out}");
    }

    #[test]
    fn a_password_account_names_the_login_it_resolved() {
        // NTU logs in with the local part, not the address. Getting that wrong is a failed
        // authentication with no explanation, so the CLI says which name it will use.
        let (store, _dir) = store();
        let out = add(&store, "b09901185@ntu.edu.tw", now()).unwrap();
        assert!(out.contains("b09901185"), "{out}");
        assert!(
            !out.contains("b09901185@ntu.edu.tw\""),
            "the login is the local part: {out}"
        );
    }

    #[test]
    fn adding_an_account_persists_its_plan_and_capabilities() {
        let (store, _dir) = store();
        add(&store, "b09901185@ntu.edu.tw", now()).unwrap();
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
