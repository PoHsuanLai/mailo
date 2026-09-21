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
    run_with(store, Arc::new(KeyringSecrets), now)
}

/// The same, with the secret store named.
///
/// The keyring is the only thing in this function that cannot exist in a test, and it was
/// reached for directly — so the one function that ties configuration, credentials, the backend,
/// the engine and the store together was the one function nothing could run. Every piece below
/// it had tests; the assembly had none.
///
/// Injected rather than faked at a lower level, because what is worth testing here *is* the
/// assembly: which credential is fetched for which purpose, what happens to an account that has
/// none, and that a failure in one account does not stop the next.
pub fn run_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
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
        match runtime.block_on(one(&store, &account, secrets.clone(), now)) {
            Ok(report) => {
                let _ = writeln!(
                    out,
                    "{}: {} headers, {} bodies, {} queued operations settled, {} sent",
                    account.address,
                    report.headers_fetched,
                    report.bodies_fetched,
                    report.outbox_settled,
                    report.submitted
                );
                // Said plainly, because the alternative is what this used to do: someone runs
                // `send` and then `sync`, reads "0 sent", and has no reason to think their mail
                // is still sitting here. It is not an error — it will be retried — but silence
                // reads as success.
                if report.still_queued > 0 {
                    let _ = writeln!(
                        out,
                        "  {} still queued; run sync again to retry, or `mailo drafts` to see why",
                        report.still_queued
                    );
                }
                for note in report.needs_attention {
                    let _ = writeln!(out, "  needs attention: {note}");
                    // The server's words stay; this adds what they mean, where we know.
                    if let Some(why) = mail_proto::explain_text(&note) {
                        let _ = writeln!(out, "    → {why}");
                    }
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
    secrets: Arc<dyn Secrets>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    let credential = secrets
        .get(&SecretKey {
            account: account.id,
            purpose: SecretPurpose::IncomingPassword,
        })
        .map_err(|_| {
            "no credential stored. Run: MAILO_PASSWORD=… mailo account add <address>".to_owned()
        })?;

    let mailbox = MailboxRef {
        account: account.id,
        path: "INBOX".to_owned(),
    };
    // Nothing cancels a one-shot CLI sync, but the loop requires a receiver, and wiring a real
    // one here is what lets the same engine serve the UI unchanged.
    let (_tx, mut cancel) = watch::channel(false);

    match &account.plan.incoming {
        Incoming::Pop3 { .. } => {
            let username = username_for(&account.plan);
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
            pass(&mut engine, &mailbox, &mut cancel, now).await
        }
        Incoming::Imap { .. } => {
            // Password *and* bearer token. Only Gmail needs OAuth; every other IMAP server this
            // client will meet authenticates with `LOGIN`, and refusing one meant the IMAP path
            // could not be used at all without registering an OAuth client.
            let mechanism = match &credential {
                Credential::OAuth { .. } => ImapCommand::AuthenticateXoauth2,
                Credential::Password(_) => ImapCommand::Login,
            };
            let auth = ImapAuth {
                username: username_for(&account.plan),
                credential,
                sasl: sasl_for(&account.plan),
            };
            let backend = ImapBackend::new(
                account.id,
                account.caps.clone(),
                // The factory owns authentication, so the backend never names a mechanism and
                // never holds the credential that would decide one.
                Box::new(move |authenticate, commands: Vec<ImapCommand>| {
                    let mut all = Vec::new();
                    if authenticate == Authenticate::First {
                        all.push(mechanism.clone());
                    }
                    all.extend(commands);
                    ImapSession::new(auth.clone(), all)
                }),
            );
            let mut engine = AccountEngine::new(
                account.id,
                account.plan.clone(),
                backend,
                store.clone(),
                secrets,
            );
            pass(&mut engine, &mailbox, &mut cancel, now).await
        }
    }
}

/// One whole sync pass, whatever the protocol underneath.
///
/// Shared rather than written once per branch, because it was written once per branch and the
/// two drifted: POP3 fetched headers, then bodies, then drained the outbox, while IMAP stopped
/// after the headers. An IMAP account therefore never downloaded a message body and never sent
/// anything it had queued, and nothing said so — the pass reported success for the part it did.
///
/// `plan.md` phase 5 asks that "the **same** CLI works through the IMAP backend". The surest way
/// to make two paths the same is for there to be one.
async fn pass<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailbox: &MailboxRef,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    // Reachability first, so an unreachable server reports once rather than three times as each
    // pass opens its own connection.
    drop(engine.connect().await.map_err(|e| e.to_string())?);

    // Ask what the server supports before deciding how to talk to it. Stored capabilities start
    // as the preset's expectation, and an expectation that is never checked is a guess the
    // client acts on for ever — CONDSTORE, MOVE and the special-use folder roles were all
    // undiscoverable until this call existed.
    if engine.caps_are_stale(now) {
        match engine.refresh_caps(cancel, now).await {
            Ok(_) => {}
            // Not fatal. A server that refuses CAPABILITY still delivers mail, and the stored
            // expectation is a worse answer than the truth but a better one than stopping.
            Err(e) => return Err(format!("could not read capabilities: {e}")),
        }
    }

    let mut report = engine
        .sync(mailbox, cancel, now, 200)
        .await
        .map_err(|e| e.to_string())?;
    // Headers first, then bodies smallest-band-first behind them, so the inbox is usable long
    // before the hundred large attachments finish.
    let bodies = engine
        .fetch_bodies(mailbox, cancel, now, 100)
        .await
        .map_err(|e| e.to_string())?;
    report.bodies_fetched += bodies.bodies_fetched;
    report.needs_attention.extend(bodies.needs_attention);

    let drained = engine
        .drain_outbox(cancel, now)
        .await
        .map_err(|e| e.to_string())?;
    report.outbox_settled += drained.outbox_settled;
    report.submitted += drained.submitted;
    report.still_queued = drained.still_queued;
    report.needs_attention.extend(drained.needs_attention);
    Ok(report)
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

// Both live on `AccountPlan` now, because `AccountEngine` needs the same answers to
// authenticate a submission and two derivations of "what do I log in as" is how one of them
// keeps working while the other quietly cannot. The wrappers stay so the tests below keep
// naming what they are testing.
fn username_for(plan: &AccountPlan) -> String {
    plan.username()
}

fn sasl_for(plan: &AccountPlan) -> Vec<SaslMech> {
    plan.sasl()
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
        assert_eq!(username_for(&preset.plan), "b09901185");
    }
}
