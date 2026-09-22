//! One sync pass, from stored configuration to a real connection.
//!
//! This is where everything meets: the preset that configured the account, the credential in the
//! keyring, the backend that speaks the protocol, and the store that keeps the result. It is
//! also the only place in the application that needs an async runtime.

use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend, Pop3Backend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, KeyringSecrets, OAuthRegistry, Secrets, SyncReport, signin};
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

/// What the server was last observed to support, for one account.
///
/// `None` when nothing has connected yet, which the caller must treat as "assume nothing" rather
/// than as an error: the window can be opened before the first sync, and refusing to act at all
/// would be worse than acting locally.
pub fn caps_of(store: &SqliteStore, account: AccountId) -> Option<AccountCaps> {
    let db = store.connection();
    let stored: Option<String> = db
        .query_row(
            "SELECT caps FROM account_caps WHERE account = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .ok();
    serde_json::from_str(&stored?).ok()
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
/// What a run reported: the lines a person reads, and the one fact a loop must act on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ran {
    pub text: String,
    /// Some account's credential was rejected. A poll loop must stop on this rather than back
    /// off: see `view::next_sync`.
    pub rejected: bool,
    /// The longest wait any server asked for. A poll loop must not knock again before it.
    pub hold: Option<std::time::Duration>,
}

/// How each configured account signs in, by address.
///
/// Beside `mailboxes_by_account` and for the same reason: `account list` should be able to say
/// what an account is waiting for without deciding it for itself. Two surfaces answering that
/// question separately is how `sync` came to tell a Google account to find a password.
pub fn auth_by_account(
    store: &SqliteStore,
) -> Result<std::collections::HashMap<String, AuthPlan>, String> {
    Ok(configured(store)?
        .into_iter()
        .map(|account| (account.address, account.plan.auth))
        .collect())
}

/// How often a background sync should run, from the accounts' own capabilities.
///
/// The shortest interval any account asks for, so an account that wants IDLE-like freshness is
/// not held to another's hourly poll. Five minutes when nothing says otherwise, which is what
/// every preset in the table already carries.
pub fn poll_interval(store: &SqliteStore) -> std::time::Duration {
    let default = std::time::Duration::from_secs(300);
    configured(store)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|a| match a.caps.watch {
            WatchMode::Poll { every } => Some(every),
            // IDLE is a long-lived connection this loop does not hold. Until it does, an account
            // that supports it is polled like any other.
            WatchMode::Idle => None,
        })
        .min()
        .unwrap_or(default)
}

pub fn run(store: Arc<SqliteStore>, now: chrono::DateTime<chrono::Utc>) -> Result<Ran, String> {
    // The registry is read once per run rather than once per account: it is deployment
    // configuration, and an edit halfway through a run producing two different client ids is
    // not a behaviour worth having.
    let registry = OAuthRegistry::load_default().map_err(|e| e.to_string())?;
    run_with(store, Arc::new(KeyringSecrets), &registry, now)
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
    registry: &OAuthRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Ran, String> {
    let accounts = configured(&store)?;
    if accounts.is_empty() {
        return Ok(Ran {
            text: "no accounts. Add one with: mailo account add <address>\n".to_owned(),
            rejected: false,
            hold: None,
        });
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;

    let mut out = String::new();
    let mut rejected = false;
    let mut hold: Option<std::time::Duration> = None;
    for account in accounts {
        match runtime.block_on(one(&store, &account, secrets.clone(), registry, now)) {
            Ok(report) => {
                rejected |= report.needs_reauth;
                if let Some(asked) = report.hold {
                    hold = Some(hold.map_or(asked, |had: std::time::Duration| had.max(asked)));
                }
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
    Ok(Ran {
        text: out,
        rejected,
        hold,
    })
}

/// The credential to authenticate with, renewed first if it is an OAuth one that has expired.
///
/// An access token lasts about an hour. Nothing did this: the stored value went straight to the
/// backend, so an OAuth account fetched mail until its first token expired and then failed on
/// every pass afterwards with an authentication error — while the refresh token that would have
/// fixed it sat unused in the same keyring entry.
async fn signed_in(
    account: &Configured,
    credential: Credential,
    secrets: &dyn Secrets,
    registry: &OAuthRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, String> {
    let AuthPlan::OAuth { issuer, .. } = &account.plan.auth else {
        return Ok(credential);
    };
    // Checked before the client is built and before anything is sent: a password account and a
    // still-valid token both leave here without touching the network.
    if matches!(
        mail_runtime::oauth::assess(&credential, now),
        mail_runtime::oauth::Freshness::Ready
    ) {
        return Ok(credential);
    }
    let Some(registration) = registry.get(*issuer) else {
        // The account was added with a client id that this installation no longer has, so the
        // token cannot be renewed and saying "authentication failed" would point at the wrong
        // thing entirely.
        return Err(format!(
            concat!(
                "the sign-in has expired and no OAuth client id is configured ",
                "for {:?}. Re-run: MAILO_OAUTH_CLIENT_ID=… mailo account add {}",
            ),
            issuer, account.address
        ));
    };
    let http = signin::http_client().map_err(|e| e.to_string())?;
    signin::renew(account.id, registration, credential, secrets, &http, now)
        .await
        .map_err(|e| format!("cannot renew the sign-in: {e}"))
}

async fn one(
    store: &Arc<SqliteStore>,
    account: &Configured,
    secrets: Arc<dyn Secrets>,
    registry: &OAuthRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    let stored = secrets
        .get(&SecretKey {
            account: account.id,
            purpose: SecretPurpose::IncomingPassword,
        })
        .map_err(|_| crate::view::no_credential(&account.address, &account.plan.auth))?;
    let credential = signed_in(account, stored, secrets.as_ref(), registry, now).await?;

    let mailboxes = to_sync(account);
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
            pass(&mut engine, &mailboxes, &mut cancel, now).await
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
            pass(&mut engine, &mailboxes, &mut cancel, now).await
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
    mailboxes: &[MailboxRef],
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

    // The inbox first and in full, then the other folders: a first sync of a large Archive must
    // not be what stands between the user and their new mail. `to_sync` puts them in that order
    // and a failure in a later one does not lose what the earlier ones fetched.
    let mut report = SyncReport::default();
    for mailbox in mailboxes {
        let one = match engine.sync(mailbox, cancel, now, 200).await {
            Ok(report) => report,
            // Named, because "cannot select" on a folder the server listed is worth seeing and
            // is not a reason to abandon the mail already in hand.
            Err(e) => {
                // Classified while the error is still typed. By the time it reaches the user it
                // is prose, and prose is not something a loop can safely decide on.
                report.saw(&e.retry());
                report
                    .needs_attention
                    .push(format!("{}: {e}", mailbox.path));
                continue;
            }
        };
        report.headers_fetched += one.headers_fetched;
        report.needs_attention.extend(one.needs_attention);

        // What the server knows and a header fetch does not carry: flags, Gmail's labels, and
        // messages that have gone. `AccountEngine::sweep` has done this since phase 3 and was
        // called from nowhere but its own tests, so every message stayed unread for ever — mail
        // read on a phone stayed bold, unread counts were the size of the mailbox, and labels
        // never arrived, since they ride the same survey.
        //
        // After the header fetch, because a sweep is about messages already held, and before the
        // bodies, so the list is right as soon as it is populated. Failures are reported and do
        // not abandon the mail already in hand, like every other step here.
        match engine.sweep(mailbox, cancel, now).await {
            Ok(swept) => report.needs_attention.extend(swept.needs_attention),
            Err(e) => {
                report.saw(&e.retry());
                report
                    .needs_attention
                    .push(format!("{}: {e}", mailbox.path));
            }
        }

        // Headers first, then bodies smallest-band-first behind them, so the inbox is usable
        // long before the hundred large attachments finish.
        match engine.fetch_bodies(mailbox, cancel, now, 100).await {
            Ok(bodies) => {
                report.bodies_fetched += bodies.bodies_fetched;
                report.needs_attention.extend(bodies.needs_attention);
            }
            Err(e) => {
                report.saw(&e.retry());
                report
                    .needs_attention
                    .push(format!("{}: {e}", mailbox.path));
            }
        }
    }

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

/// The mailboxes the next pass would fetch, per account, by address.
///
/// Reported by `mailo account list`, because "why is my Sent folder empty" is a question a user
/// asks and the answer is a fact this client knows: only these folders are fetched. Exposed
/// rather than duplicated so a test can ask the code rather than restate its rules —
/// CONVENTIONS §"An assertion that was already true proves nothing", second half.
pub fn mailboxes_by_account(store: &SqliteStore) -> Result<Vec<(String, Vec<String>)>, String> {
    Ok(configured(store)?
        .into_iter()
        .map(|account| {
            let paths = to_sync(&account).into_iter().map(|m| m.path).collect();
            (account.address, paths)
        })
        .collect())
}

/// Which mailboxes a pass should fetch, in the order it should fetch them.
///
/// The inbox first and always, then `Sent` where the server named one.
///
/// Only the inbox was ever fetched, which nothing said and nothing tested: a user who sends mail
/// from their phone opened Sent and found only what this client had sent, and mail archived
/// elsewhere vanished from the inbox without appearing in Archive. `Spam` is deliberately not
/// here — downloading the spam folder to populate a place nobody opens costs a first sync twice
/// over — and neither is `Drafts`, whose server copies are other clients' half-written mail and
/// would collide with this one's outbox.
///
/// POP3 has one mailbox by construction, and a server that answered no roles gets the inbox
/// alone, which is what happened before this existed.
fn to_sync(account: &Configured) -> Vec<MailboxRef> {
    let mut out = vec![MailboxRef {
        account: account.id,
        path: "INBOX".to_owned(),
    }];
    // `Sent` and not `Archive`, which is not safe yet. On Gmail the same message is in INBOX
    // *and* in All Mail, and `Message.mailbox` is one role: a pass over All Mail would mark a
    // message that is in the inbox as archived and it would leave the list. That needs the
    // message to carry a set of mailboxes rather than one, which is a domain change and not a
    // sync one. A message in Sent is not also in the inbox, so Sent is safe today.
    for role in [MailboxRole::Sent] {
        if let Some(path) = account.caps.folders.path(role)
            && !path.eq_ignore_ascii_case("INBOX")
        {
            out.push(MailboxRef {
                account: account.id,
                path: path.to_owned(),
            });
        }
    }
    out
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
        .unwrap()
        .text;
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
