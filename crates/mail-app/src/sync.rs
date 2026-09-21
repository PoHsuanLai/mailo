//! One sync pass, from stored configuration to a real connection.
//!
//! This is where everything meets: the preset that configured the account, the credential in the
//! keyring, the backend that speaks the protocol, and the store that keeps the result. It is
//! also the only place in the application that needs an async runtime.

use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend, Pop3Backend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, KeyringSecrets, Secrets, SyncReport};
use mail_store::SqliteStore;
use std::fmt::Write as _;
use std::sync::Arc;
use tokio::sync::watch;

/// Everything stored about one account.
struct Configured {
    id: AccountId,
    address: String,
    plan: AccountPlan,
    caps: AccountCaps,
}

/// Read the accounts back out of the store.
fn configured(store: &SqliteStore) -> Result<Vec<Configured>, String> {
    let db = store.connection();
    let mut stmt = db
        .prepare(
            "SELECT a.id, a.address, a.plan, c.caps
             FROM accounts a LEFT JOIN account_caps c ON c.account = a.id
             ORDER BY a.created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        let (id, address, plan, caps) = row.map_err(|e| e.to_string())?;
        let Ok(uuid) = id.parse() else { continue };
        let plan: AccountPlan = serde_json::from_str(&plan)
            .map_err(|e| format!("{address}: stored plan is unreadable: {e}"))?;
        let caps: AccountCaps = match caps {
            Some(text) => serde_json::from_str(&text)
                .map_err(|e| format!("{address}: stored capabilities are unreadable: {e}"))?,
            // No capabilities yet means nothing has connected. The expected ones from the
            // preset are a starting point, not a claim about the server.
            None => {
                return Err(format!(
                    "{address}: no capabilities recorded. Something has gone wrong with setup."
                ));
            }
        };
        out.push(Configured {
            id: AccountId::from_uuid(uuid),
            address,
            plan,
            caps,
        });
    }
    Ok(out)
}

/// Sync every configured account that has a credential, reporting what happened.
///
/// Accounts without one are skipped with a reason rather than failing the run: having one
/// account that needs attention should not stop the others from fetching mail.
pub fn run(store: Arc<SqliteStore>, now: chrono::DateTime<chrono::Utc>) -> Result<String, String> {
    let accounts = configured(&store)?;
    if accounts.is_empty() {
        return Ok("no accounts. Add one with: mailo account add <address>\n".to_owned());
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;

    let mut out = String::new();
    for account in accounts {
        match runtime.block_on(one(&store, &account, now)) {
            Ok(report) => {
                let _ = writeln!(
                    out,
                    "{}: {} headers, {} bodies, {} queued operations settled",
                    account.address,
                    report.headers_fetched,
                    report.bodies_fetched,
                    report.outbox_settled
                );
                for note in report.needs_attention {
                    let _ = writeln!(out, "  needs attention: {note}");
                }
            }
            Err(why) => {
                let _ = writeln!(out, "{}: {why}", account.address);
            }
        }
    }
    Ok(out)
}

async fn one(
    store: &Arc<SqliteStore>,
    account: &Configured,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    let credential = KeyringSecrets
        .get(&SecretKey {
            account: account.id,
            purpose: SecretPurpose::IncomingPassword,
        })
        .map_err(|_| {
            "no credential stored. Run: MAILO_PASSWORD=… mailo account add <address>".to_owned()
        })?;

    let secrets: Arc<dyn Secrets> = Arc::new(KeyringSecrets);
    let mailbox = MailboxRef {
        account: account.id,
        path: "INBOX".to_owned(),
    };
    // Nothing cancels a one-shot CLI sync, but the loop requires a receiver, and wiring a real
    // one here is what lets the same engine serve the UI unchanged.
    let (_tx, mut cancel) = watch::channel(false);

    match &account.plan.incoming {
        Incoming::Pop3 { .. } => {
            let username = username_for(&account.plan, &account.address);
            let Credential::Password(password) = credential else {
                return Err("POP3 needs a password credential".to_owned());
            };
            let sasl = sasl_for(&account.plan);
            let backend = Pop3Backend::new(
                account.id,
                account.caps.clone(),
                Box::new(move |auth, commands| {
                    // The factory owns authentication, because it is the only thing holding the
                    // credential and the only thing that knows the mechanism.
                    let mut all = Vec::new();
                    if auth == Authenticate::First {
                        all.extend(auth_prelude(&sasl));
                    }
                    all.extend(commands);
                    Pop3Session::new(username.clone(), password.clone(), all)
                }),
            );
            let mut engine = AccountEngine::new(
                account.id,
                account.plan.clone(),
                backend,
                store.clone(),
                secrets,
            );
            let mut transport = engine.connect().await.map_err(|e| e.to_string())?;
            let mut report = engine
                .sync(&mailbox, &mut transport, &mut cancel, now, 200)
                .await
                .map_err(|e| e.to_string())?;
            // Headers first, then bodies smallest-band-first behind them, so the inbox is
            // usable long before the hundred large attachments finish.
            let bodies = engine
                .fetch_bodies(&mailbox, &mut transport, &mut cancel, now, 100)
                .await
                .map_err(|e| e.to_string())?;
            report.bodies_fetched += bodies.bodies_fetched;
            report.needs_attention.extend(bodies.needs_attention);
            let drained = engine
                .drain_outbox(&mut transport, &mut cancel, now)
                .await
                .map_err(|e| e.to_string())?;
            report.outbox_settled += drained.outbox_settled;
            report.needs_attention.extend(drained.needs_attention);
            Ok(report)
        }
        Incoming::Imap { .. } => {
            let Credential::OAuth { .. } = credential else {
                return Err(
                    "IMAP here expects an OAuth credential; password IMAP is not wired up"
                        .to_owned(),
                );
            };
            let auth = ImapAuth {
                username: account.address.clone(),
                credential,
                sasl: sasl_for(&account.plan),
            };
            let backend = ImapBackend::new(
                account.id,
                account.caps.clone(),
                Box::new(move |commands: Vec<ImapCommand>| {
                    ImapSession::new(auth.clone(), commands)
                }),
            );
            let mut engine = AccountEngine::new(
                account.id,
                account.plan.clone(),
                backend,
                store.clone(),
                secrets,
            );
            let mut transport = engine.connect().await.map_err(|e| e.to_string())?;
            engine
                .sync(&mailbox, &mut transport, &mut cancel, now, 200)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// The commands that authenticate a POP3 session, given the mechanisms on offer.
fn auth_prelude(sasl: &[SaslMech]) -> Vec<Pop3Command> {
    if sasl.contains(&SaslMech::Plain) {
        vec![Pop3Command::AuthPlain]
    } else {
        // USER/PASS is the universal fallback and the only thing some servers implement.
        vec![Pop3Command::User, Pop3Command::Pass]
    }
}

fn username_for(plan: &AccountPlan, address: &str) -> String {
    match &plan.auth {
        AuthPlan::Password { username, .. } => username.resolve(address),
        AuthPlan::OAuth { .. } => address.to_owned(),
    }
}

fn sasl_for(plan: &AccountPlan) -> Vec<SaslMech> {
    match &plan.auth {
        AuthPlan::Password { sasl, .. } => sasl.clone(),
        AuthPlan::OAuth { .. } => vec![SaslMech::XOauth2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncing_with_no_accounts_explains_how_to_add_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        let out = run(
            store,
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        )
        .unwrap();
        assert!(out.contains("mailo account add"), "{out}");
    }

    #[test]
    fn the_auth_prelude_prefers_sasl_but_falls_back() {
        // NTU offers PLAIN only; some older servers offer neither and only understand USER/PASS.
        assert_eq!(
            auth_prelude(&[SaslMech::Plain]),
            vec![Pop3Command::AuthPlain]
        );
        assert_eq!(
            auth_prelude(&[]),
            vec![Pop3Command::User, Pop3Command::Pass]
        );
    }

    #[test]
    fn a_password_login_resolves_to_the_local_part_where_the_preset_says_so() {
        // Getting this wrong is an authentication failure with no explanation.
        let preset = mail_domain::presets::preset_for(
            "b09901185@ntu.edu.tw",
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        )
        .expect("ntu is a known domain");
        assert_eq!(
            username_for(&preset.plan, "b09901185@ntu.edu.tw"),
            "b09901185"
        );
    }
}
